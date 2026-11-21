use opensdl_assets::{AssetCache, AssetReference, RegistryClient, RegistryCredentials};

pub async fn acquire(
    cache: &AssetCache,
    references: Vec<AssetReference>,
) -> anyhow::Result<std::collections::HashSet<String>> {
    use anyhow::Context;
    let mut pulls = tokio::task::JoinSet::new();
    let mut requested = std::collections::HashSet::new();
    for reference in references {
        let digest = reference.digest().context("scene requires pinned assets")?;
        if !requested.insert(digest.to_owned()) || cache.package(digest).is_ok() {
            continue;
        }
        let cache = cache.clone();
        pulls.spawn(async move {
            RegistryClient::for_reference(&reference)
                .pull(&reference, &cache, &RegistryCredentials::Anonymous, false)
                .await
        });
        if pulls.len() == 4 {
            pulls
                .join_next()
                .await
                .context("asset download missing")???;
        }
    }
    while let Some(result) = pulls.join_next().await {
        result??;
    }
    Ok(requested)
}
