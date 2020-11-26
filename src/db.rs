use diesel::SqliteConnection;
use diesel::prelude::*;
use crossbeam::channel;
use std::thread::{JoinHandle};
use futures::channel::oneshot;
use once_cell::sync::{Lazy, OnceCell};
use std::borrow::Cow;
use diesel::result::DatabaseErrorKind;
use std::path::Path;

type ConnFunPtr = Box<dyn FnOnce(&mut SqliteConnection) + Send + 'static>;

pub struct DBService {
    task_pool: channel::Sender<ConnFunPtr>,
    tl_pool: thread_local::ThreadLocal<channel::Sender<ConnFunPtr>>,
    runner: Option<JoinHandle<()>>,
}

impl From<SqliteConnection> for DBService {
    fn from(conn: SqliteConnection) -> Self {
        let (send, recv): (channel::Sender<ConnFunPtr>, _) = channel::unbounded();
        let runner = std::thread::spawn(move || {
            let mut c = conn;
            recv.into_iter()
                .for_each(|f| f(&mut c))
        });
        DBService { task_pool: send, tl_pool: thread_local::ThreadLocal::new(), runner: Some(runner) }
    }
}

impl Drop for DBService {
    fn drop(&mut self) {
        self.runner.take().map(|j| j.join().unwrap());
    }
}

impl DBService {
    pub async fn with_conn<F, T>(&self, f: F) -> T
        where F: FnOnce(&mut SqliteConnection) -> T + Send + 'static,
              T: Send + 'static {
        let (send, recv) = oneshot::channel();
        self.local_pool().await.send(
            Box::new(|c| {
                send.send(f(c)).ok().unwrap()
            })
        ).unwrap();

        recv.await.unwrap()
    }

    async fn local_pool(&self) -> &channel::Sender<ConnFunPtr> {
        self.tl_pool.get_or(|| self.task_pool.clone())
    }
}

pub fn establish_connection(path: impl AsRef<str>) -> SqliteConnection {
    let pd: &Path = path.as_ref().as_ref();
    let p = pd.parent().expect("Can't install a sqlite database in the root directory safely.");
    std::fs::create_dir_all(p).unwrap();
    debug!("Opening sqlite database at {}", path.as_ref());
    SqliteConnection::establish(path.as_ref()).unwrap()
}

static DEFAULT_DB_PATH: Lazy<String> = Lazy::new(
    || {
        dirs::data_local_dir().map(|p| p.join("aliasd").join("alias.sqlite"))
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
    }
);

pub fn db_path() -> Cow<'static, str> {
    std::env::var("DATABASE_URL").map(|s| Cow::Owned(s)).unwrap_or(Cow::from(DEFAULT_DB_PATH.as_str()))
}

pub static DB_SERVICE: OnceCell<DBService> = OnceCell::new();

pub fn init_service(conn: SqliteConnection) {
    DB_SERVICE.get_or_init(|| DBService::from(conn));
}

pub fn conn() -> &'static DBService {
    DB_SERVICE.get().expect("DB service was not initialized")
}

pub trait DBError {
    fn is_uniqueness_violation(&self) -> bool;
}

impl DBError for diesel::result::Error {
    fn is_uniqueness_violation(&self) -> bool {
        if let diesel::result::Error::DatabaseError(k, _) = self {
            matches!(k, DatabaseErrorKind::UniqueViolation)
        } else {
            false
        }
    }
}
