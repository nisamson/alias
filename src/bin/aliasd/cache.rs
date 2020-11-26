use lru::LruCache;
use once_cell::sync::Lazy;
use tokio::sync::{Mutex};
use alias::db::conn;
use diesel::{QueryDsl, ExpressionMethods, RunQueryDsl};
use std::sync::Arc;
use diesel::result::Error;

const CACHE_SIZE: usize = 4096;

static ALIAS_CACHE: Lazy<Mutex<LruCache<String, String>>> = Lazy::new(
    || Mutex::new(LruCache::new(CACHE_SIZE))
);

#[derive(Debug, thiserror::Error)]
pub enum AliasSearchFailure {
    #[error("No such alias was present")]
    NoSuchAlias,
    #[error("Ran into a problem running the query.")]
    Sql(diesel::result::Error)
}

impl From<diesel::result::Error> for AliasSearchFailure {
    fn from(e: Error) -> Self {
        match e {
            diesel::result::Error::NotFound => AliasSearchFailure::NoSuchAlias,
            _ => AliasSearchFailure::Sql(e)
        }
    }
}

pub async fn get_alias(s: impl Into<String>) -> Result<String, AliasSearchFailure> {
    let s = s.into();

    let mut cache_g = ALIAS_CACHE.lock().await;

    let res = cache_g.get(&s);

    if let Some(s) = res {
        return Ok(s.clone());
    }

    std::mem::drop(cache_g);

    let s = Arc::<String>::from(s);
    let qs = s.clone();
    let dest: String = conn().with_conn(move |c| {
        use ::alias::schema::aliases::dsl::*;
        aliases.select(destination)
            .filter(alias.eq(&*qs))
            .get_result(c)
            .map_err(AliasSearchFailure::from)
    }).await?;

    let mut cache_g = ALIAS_CACHE.lock().await;
    cache_g.put(Arc::try_unwrap(s).unwrap(), dest.clone());

    Ok(dest)
}

pub async fn evict_alias(s: impl Into<String>) {
    let s = s.into();
    let mut cache_g = ALIAS_CACHE.lock().await;
    cache_g.pop(&s);
}