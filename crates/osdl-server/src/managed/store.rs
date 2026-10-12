use anyhow::{bail, Context};
use opensdl_assets::AssetCache;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InstanceState {
    Creating,
    Created,
    Starting,
    Running,
    Stopping,
    Exited,
    Failed,
    Removing,
}
impl InstanceState {
    pub(crate) fn parse(value: &str) -> anyhow::Result<Self> {
        Ok(match value {
            "creating" => Self::Creating,
            "created" => Self::Created,
            "starting" => Self::Starting,
            "running" => Self::Running,
            "stopping" => Self::Stopping,
            "exited" => Self::Exited,
            "failed" => Self::Failed,
            "removing" => Self::Removing,
            _ => bail!("invalid managed instance state"),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InstanceInfo {
    pub name: String,
    pub manifest_digest: String,
    pub created_at: i64,
    pub state: InstanceState,
    pub generation: i64,
    pub pid: Option<u32>,
    pub endpoint: Option<String>,
    pub error: Option<String>,
    pub data_dir: PathBuf,
    pub log_path: PathBuf,
}

pub(crate) struct Record {
    pub info: InstanceInfo,
    pub config: String,
    pub token: String,
}

#[derive(Clone)]
pub(crate) struct Store {
    pub cache: AssetCache,
}
impl Store {
    pub fn new(cache: AssetCache) -> anyhow::Result<Self> {
        owned_directory(&cache.root().join("instances"))?;
        owned_directory(&cache.root().join("instance-locks"))?;
        Ok(Self { cache })
    }
    pub fn directory(&self, name: &str) -> PathBuf {
        self.cache.root().join("instances").join(name)
    }
    pub fn connect(&self) -> anyhow::Result<Connection> {
        let connection = Connection::open(self.cache.catalog_path())?;
        connection.busy_timeout(std::time::Duration::from_secs(10))?;
        Ok(connection)
    }
    pub fn read(&self, name: &str) -> anyhow::Result<Record> {
        validate_name(name)?;
        let connection = self.connect()?;
        let row=connection.query_row("SELECT digest,created_at,state,generation,config,token,pid,endpoint,error FROM managed_instance WHERE name=?1",[name],|row| {
            Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,String>(2)?,row.get::<_,i64>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,Option<u32>>(6)?,row.get::<_,Option<String>>(7)?,row.get::<_,Option<String>>(8)?))
        }).optional()?.with_context(|| format!("managed instance '{name}' does not exist"))?;
        let directory = self.directory(name);
        Ok(Record {
            info: InstanceInfo {
                name: name.to_owned(),
                manifest_digest: row.0,
                created_at: row.1,
                state: InstanceState::parse(&row.2)?,
                generation: row.3,
                pid: row.6,
                endpoint: row.7,
                error: row.8,
                data_dir: directory.join("data"),
                log_path: directory.join("instance.log"),
            },
            config: row.4,
            token: row.5,
        })
    }
    pub fn names(&self) -> anyhow::Result<Vec<String>> {
        let connection = self.connect()?;
        let mut statement =
            connection.prepare("SELECT name FROM managed_instance ORDER BY name")?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(names)
    }
    pub fn reserve(&self, name: &str, digest: &str) -> anyhow::Result<()> {
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        tx.execute("INSERT INTO managed_instance(name,digest,created_at,state,generation,config,token) VALUES(?1,?2,?3,'creating',0,'','')",params![name,digest,now()?]).with_context(||format!("cannot reserve instance '{name}' (name may already exist)"))?;
        tx.execute(
            "INSERT INTO pin(owner,digest) VALUES(?1,?2)",
            params![format!("instance:{name}"), digest],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn prepared(&self, name: &str, config: &str) -> anyhow::Result<()> {
        let changed=self.connect()?.execute("UPDATE managed_instance SET state='created',config=?2 WHERE name=?1 AND state='creating'",params![name,config])?;
        if changed != 1 {
            bail!("instance preparation lost its reservation");
        }
        Ok(())
    }
    pub fn generation(&self, name: &str, token: &str) -> anyhow::Result<i64> {
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        tx.execute("UPDATE instance_sequence SET value=value+1", [])?;
        let generation: i64 =
            tx.query_row("SELECT value FROM instance_sequence", [], |row| row.get(0))?;
        if tx.execute("UPDATE managed_instance SET state='starting',generation=?3,token=?2,pid=NULL,endpoint=NULL,error=NULL WHERE name=?1",params![name,token,generation])? != 1 { bail!("instance no longer exists"); }
        tx.commit()?;
        Ok(generation)
    }
    pub fn running(
        &self,
        name: &str,
        generation: i64,
        pid: u32,
        endpoint: &str,
    ) -> anyhow::Result<()> {
        if self.connect()?.execute("UPDATE managed_instance SET state='running',pid=?3,endpoint=?4 WHERE name=?1 AND generation=?2 AND state='starting'",params![name,generation,pid,endpoint])?!=1 { bail!("instance start generation was superseded"); }
        Ok(())
    }
    pub fn settle(
        &self,
        name: &str,
        generation: i64,
        state: &str,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        self.connect()?.execute("UPDATE managed_instance SET state=?3,error=?4,token='',endpoint=NULL,pid=NULL WHERE name=?1 AND generation=?2",params![name,generation,state,error])?;
        Ok(())
    }
    pub fn stopping(&self, name: &str, generation: i64) -> anyhow::Result<()> {
        self.connect()?.execute("UPDATE managed_instance SET state='stopping' WHERE name=?1 AND generation=?2 AND state='running'",params![name,generation])?;
        Ok(())
    }
    pub fn delete(&self, name: &str) -> anyhow::Result<()> {
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        tx.execute("DELETE FROM managed_instance WHERE name=?1", [name])?;
        tx.execute(
            "DELETE FROM pin WHERE owner=?1",
            [format!("instance:{name}")],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn command_lock(&self, name: &str) -> anyhow::Result<File> {
        validate_name(name)?;
        let file = regular_lock(
            &self
                .cache
                .root()
                .join("instance-locks")
                .join(format!("{name}.lock")),
        )?;
        file.lock()?;
        Ok(file)
    }
    pub fn process_lock(&self, name: &str) -> anyhow::Result<File> {
        validate_name(name)?;
        owned_directory(&self.directory(name))?;
        regular_lock(&self.directory(name).join("process.lock"))
    }
    pub fn worker_lock(&self, name: &str) -> anyhow::Result<File> {
        validate_name(name)?;
        let directory = self.directory(name);
        let metadata = directory.symlink_metadata()?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!("worker directory is unavailable");
        }
        let path = directory.join("process.lock");
        regular_file(&path)?;
        Ok(fs::OpenOptions::new().read(true).write(true).open(path)?)
    }
}

pub(crate) fn validate_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty()
        || name.len() > 63
        || !name.as_bytes()[0].is_ascii_lowercase()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        bail!("instance name must start with a lowercase letter and contain 1–63 lowercase letters, digits, '-' or '_'");
    }
    Ok(())
}
pub(crate) fn owned_directory(path: &Path) -> anyhow::Result<()> {
    if let Ok(metadata) = path.symlink_metadata() {
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!(
                "instance directory is not an owned directory: {}",
                path.display()
            );
        }
        return Ok(());
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}
fn regular_lock(path: &Path) -> anyhow::Result<File> {
    regular_file(path)?;
    Ok(fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?)
}

pub(crate) fn regular_file(path: &Path) -> anyhow::Result<()> {
    match path.symlink_metadata() {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => bail!(
            "managed file must be a regular owned file: {}",
            path.display()
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
pub(crate) fn now() -> anyhow::Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}
pub(crate) fn idle(file: &File) -> anyhow::Result<bool> {
    if acquire_idle(file)? {
        file.unlock()?;
        Ok(true)
    } else {
        Ok(false)
    }
}
pub(crate) fn acquire_idle(file: &File) -> anyhow::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(std::fs::TryLockError::WouldBlock) => Ok(false),
        Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_old_worker_cannot_settle_the_new_generation() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(AssetCache::open(temp.path()).unwrap()).unwrap();
        let digest = format!("sha256:{}", "a".repeat(64));
        store.reserve("demo", &digest).unwrap();
        store.prepared("demo", "engine: kinematic\n").unwrap();
        let old = store.generation("demo", "first-secret").unwrap();
        let current = store.generation("demo", "second-secret").unwrap();
        store
            .settle("demo", old, "failed", Some("old worker"))
            .unwrap();
        let record = store.read("demo").unwrap();
        assert_eq!(record.info.generation, current);
        assert!(matches!(record.info.state, InstanceState::Starting));
        assert_eq!(record.token, "second-secret");
        let public = serde_json::to_string(&record.info).unwrap();
        assert!(!public.contains("secret"));
    }
    #[test]
    fn deleting_and_recreating_a_name_cannot_reuse_a_worker_generation() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(AssetCache::open(temp.path()).unwrap()).unwrap();
        let digest = format!("sha256:{}", "a".repeat(64));
        store.reserve("same", &digest).unwrap();
        let old = store.generation("same", "old").unwrap();
        store.delete("same").unwrap();
        store.reserve("same", &digest).unwrap();
        let current = store.generation("same", "new").unwrap();
        assert!(current > old);
        store.settle("same", old, "failed", None).unwrap();
        assert_eq!(store.read("same").unwrap().token, "new");
    }
    #[test]
    fn a_delayed_worker_cannot_recreate_a_removed_directory() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(AssetCache::open(temp.path()).unwrap()).unwrap();
        assert!(store.worker_lock("gone").is_err());
        assert!(!store.directory("gone").exists());
    }
    #[test]
    fn removal_retries_after_files_are_deleted_but_the_catalog_commit_fails() {
        let temp = tempfile::tempdir().unwrap();
        let cache = AssetCache::open(temp.path()).unwrap();
        let store = Store::new(cache.clone()).unwrap();
        store
            .reserve("retry", &format!("sha256:{}", "a".repeat(64)))
            .unwrap();
        store.prepared("retry", "engine: kinematic\n").unwrap();
        owned_directory(&store.directory("retry").join("data")).unwrap();
        std::fs::write(store.directory("retry").join("data/retained"), b"owned").unwrap();
        store.connect().unwrap().execute_batch("CREATE TRIGGER fail_delete BEFORE DELETE ON managed_instance BEGIN SELECT RAISE(ABORT, 'injected deletion failure'); END;").unwrap();
        let host = super::super::ManagedHost::open(cache).unwrap();
        assert!(host.remove("retry").is_err());
        assert!(matches!(
            store.read("retry").unwrap().info.state,
            InstanceState::Removing
        ));
        assert!(!store.directory("retry").exists());
        store
            .connect()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_delete;")
            .unwrap();
        host.remove("retry").unwrap();
        assert!(store.read("retry").is_err());
        let roots: i64 = store
            .connect()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM pin", [], |row| row.get(0))
            .unwrap();
        assert_eq!(roots, 0);
        assert!(!store.directory("retry").exists());
    }
}
