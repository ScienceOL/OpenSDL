//! Transactional metadata for the local content-addressed resource store.
use std::collections::HashSet;
use std::fs;
use std::path::Path;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};

const APPLICATION_ID: u32 = 0x4f53444c;
use serde::Serialize;

use crate::{AssetError, AssetReference, PackageDescriptor};

#[derive(Debug, Serialize)]
pub struct AssetSummary {
    pub manifest_digest: String,
    pub name: String,
    pub version: String,
    pub references: Vec<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct PruneReport {
    pub packages: Vec<String>,
    pub blobs: Vec<String>,
    pub reclaimed_bytes: u64,
}

pub(crate) struct Catalog {
    connection: Connection,
}

impl Catalog {
    pub(crate) fn open(path: &Path) -> Result<Self, AssetError> {
        match path.symlink_metadata() {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(AssetError::invalid(
                        "catalog must be a store-owned regular file",
                    ));
                }
                if metadata.len() > 0 {
                    let existing =
                        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                    let application_id: u32 =
                        existing.query_row("PRAGMA application_id", [], |row| row.get(0))?;
                    if application_id != APPLICATION_ID {
                        return Err(AssetError::invalid(
                            "directory contains an unrelated catalog database",
                        ));
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(AssetError::read(path, error)),
        }
        let connection = Connection::open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .map_err(|error| AssetError::read(path, error))?;
        }
        connection.busy_timeout(std::time::Duration::from_secs(10))?;
        let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > 3 {
            return Err(AssetError::invalid(
                "unsupported local resource catalog version",
            ));
        }
        if version == 0 {
            connection.execute_batch(&format!(
                "BEGIN IMMEDIATE;
                PRAGMA application_id = {APPLICATION_ID};
                CREATE TABLE package (digest TEXT PRIMARY KEY, descriptor TEXT NOT NULL);
                CREATE TABLE reference (name TEXT PRIMARY KEY, digest TEXT NOT NULL);
                CREATE TABLE pin (owner TEXT PRIMARY KEY, digest TEXT NOT NULL);
                PRAGMA user_version = 1;
                COMMIT;"
            ))?;
        }
        if version < 2 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE managed_instance (
                    name TEXT PRIMARY KEY, digest TEXT NOT NULL, created_at INTEGER NOT NULL,
                    state TEXT NOT NULL, generation INTEGER NOT NULL, config TEXT NOT NULL,
                    token TEXT NOT NULL, pid INTEGER, endpoint TEXT, error TEXT);
                PRAGMA user_version = 2; COMMIT;",
            )?;
        }
        if version < 3 {
            connection.execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE instance_sequence(value INTEGER NOT NULL);
                INSERT INTO instance_sequence SELECT COALESCE(MAX(generation),0) FROM managed_instance;
                PRAGMA user_version = 3; COMMIT;",
            )?;
        }
        Ok(Self { connection })
    }

    pub(crate) fn admit(
        &mut self,
        package: &PackageDescriptor,
        reference: Option<&AssetReference>,
    ) -> Result<(), AssetError> {
        let descriptor =
            serde_json::to_string(package).map_err(|e| AssetError::invalid(e.to_string()))?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO package(digest, descriptor) VALUES(?1,?2) ON CONFLICT(digest) DO NOTHING",
            params![package.manifest_digest, descriptor],
        )?;
        if let Some(reference) = reference {
            transaction.execute("INSERT INTO reference(name,digest) VALUES(?1,?2) ON CONFLICT(name) DO UPDATE SET digest=excluded.digest", params![reference.to_string(), package.manifest_digest])?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn resolve(&self, reference: &AssetReference) -> Result<String, AssetError> {
        if let Some(digest) = reference.digest() {
            self.require_package(digest)?;
            return Ok(digest.to_owned());
        }
        let digest: String = self
            .connection
            .query_row(
                "SELECT digest FROM reference WHERE name=?1",
                [reference.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| AssetError::NotFound(reference.to_string()))?;
        self.require_package(&digest).map_err(|error| match error {
            AssetError::NotFound(_) => {
                AssetError::invalid("reference points to a package missing from inventory")
            }
            other => other,
        })?;
        Ok(digest)
    }

    pub(crate) fn require_package(&self, digest: &str) -> Result<(), AssetError> {
        let admitted: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM package WHERE digest=?1)",
            [digest],
            |row| row.get(0),
        )?;
        if !admitted {
            return Err(AssetError::NotFound(digest.to_owned()));
        }
        Ok(())
    }

    pub(crate) fn packages(&self) -> Result<Vec<PackageDescriptor>, AssetError> {
        let mut query = self
            .connection
            .prepare("SELECT descriptor FROM package ORDER BY digest")?;
        let rows = query.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| {
            serde_json::from_str(&row?)
                .map_err(|e| AssetError::invalid(format!("invalid catalog package: {e}")))
        })
        .collect()
    }

    pub(crate) fn list(&self) -> Result<Vec<AssetSummary>, AssetError> {
        self.packages()?
            .into_iter()
            .map(|package| {
                let mut query = self
                    .connection
                    .prepare("SELECT name FROM reference WHERE digest=?1 ORDER BY name")?;
                let references = query
                    .query_map([&package.manifest_digest], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                let size_bytes = std::iter::once(&package.config)
                    .chain(package.layers.iter())
                    .try_fold(package.manifest_size, |total, descriptor| {
                        total
                            .checked_add(descriptor.size)
                            .ok_or_else(|| AssetError::invalid("catalog package size overflow"))
                    })?;
                Ok(AssetSummary {
                    manifest_digest: package.manifest_digest,
                    name: package.asset.metadata.name,
                    version: package.asset.metadata.version,
                    references,
                    size_bytes,
                })
            })
            .collect()
    }

    pub(crate) fn pin(&self, owner: &str, digest: &str) -> Result<(), AssetError> {
        if owner.trim().is_empty() {
            return Err(AssetError::invalid("resource owner must not be empty"));
        }
        self.connection.execute("INSERT INTO pin(owner,digest) VALUES(?1,?2) ON CONFLICT(owner) DO UPDATE SET digest=excluded.digest", params![owner,digest])?;
        Ok(())
    }

    pub(crate) fn unpin(&self, owner: &str) -> Result<(), AssetError> {
        self.connection
            .execute("DELETE FROM pin WHERE owner=?1", [owner])?;
        Ok(())
    }

    pub(crate) fn remove(&self, selector: &str) -> Result<(), AssetError> {
        if selector.starts_with("sha256:") {
            crate::reference::validate_digest(selector)?;
            let occupied: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM reference WHERE digest=?1 UNION ALL SELECT 1 FROM pin WHERE digest=?1)", [selector], |row| row.get(0))?;
            if occupied {
                return Err(AssetError::invalid(
                    "package is referenced or occupied; remove its references and instances first",
                ));
            }
            if self
                .connection
                .execute("DELETE FROM package WHERE digest=?1", [selector])?
                == 0
            {
                return Err(AssetError::invalid("package is not in the local inventory"));
            }
        } else {
            let reference = AssetReference::parse(selector)?;
            if self.connection.execute(
                "DELETE FROM reference WHERE name=?1",
                [reference.to_string()],
            )? == 0
            {
                return Err(AssetError::invalid(
                    "reference is not in the local inventory",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn prune(
        &mut self,
        blobs: &Path,
        dry_run: bool,
        verify: impl Fn(&str) -> Result<PackageDescriptor, AssetError>,
    ) -> Result<PruneReport, AssetError> {
        let mut query = self
            .connection
            .prepare("SELECT digest FROM reference UNION SELECT digest FROM pin")?;
        let roots = query
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<HashSet<_>, _>>()?;
        drop(query);
        let admitted = self.packages()?;
        let admitted_digests: HashSet<_> = admitted
            .iter()
            .map(|package| package.manifest_digest.as_str())
            .collect();
        if roots
            .iter()
            .any(|digest| !admitted_digests.contains(digest.as_str()))
        {
            return Err(AssetError::invalid("catalog has a referenced or occupied package missing from inventory; refusing garbage collection"));
        }
        let mut packages = Vec::new();
        let mut reachable = HashSet::new();
        for package in admitted {
            if roots.contains(&package.manifest_digest) {
                let package = verify(&package.manifest_digest)?;
                reachable.insert(package.manifest_digest);
                reachable.insert(package.config.digest);
                reachable.extend(package.layers.into_iter().map(|layer| layer.digest));
            } else {
                packages.push(package.manifest_digest);
            }
        }
        let mut report = PruneReport {
            packages,
            blobs: Vec::new(),
            reclaimed_bytes: 0,
        };
        for entry in fs::read_dir(blobs).map_err(|e| AssetError::read(blobs, e))? {
            let entry = entry.map_err(|e| AssetError::read(blobs, e))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let digest = format!("sha256:{name}");
            if crate::reference::validate_digest(&digest).is_err() {
                continue;
            }
            if !reachable.contains(&digest) {
                let metadata = entry
                    .path()
                    .symlink_metadata()
                    .map_err(|e| AssetError::read(&entry.path(), e))?;
                if !metadata.is_file() {
                    return Err(AssetError::invalid("unexpected non-file in blob store"));
                }
                report.reclaimed_bytes = report
                    .reclaimed_bytes
                    .checked_add(metadata.len())
                    .ok_or_else(|| AssetError::invalid("prune byte count overflow"))?;
                report.blobs.push(digest);
            }
        }
        report.blobs.sort();
        if !dry_run {
            let transaction = self.connection.transaction()?;
            for digest in &report.packages {
                transaction.execute("DELETE FROM package WHERE digest=?1", [digest])?;
            }
            transaction.commit()?;
            for digest in &report.blobs {
                let path = blobs.join(crate::reference::validate_digest(digest)?);
                fs::remove_file(&path).map_err(|e| AssetError::read(&path, e))?;
            }
        }
        Ok(report)
    }
}
