#[macro_use]
extern crate diesel_migrations;
#[macro_use]
extern crate rocket;
#[macro_use]
extern crate serde_json;

use std::io::Cursor;
use std::sync::Arc;

use clap::{App, AppSettings, Arg};
use diesel::{ExpressionMethods, QueryResult, RunQueryDsl};
use rocket::{Config, Response};
use rocket::http::{ContentType, Cookie, CookieJar, Status};
use rocket_contrib::helmet::SpaceHelmet;
use rocket_contrib::json::Json;
use tracing::Level;
use url::Url;

use alias::*;
use alias::db::conn;
use alias::model::{AliasForm, Claims, Login, LoginFailure};

use crate::cache::AliasSearchFailure;


mod users;
mod cache;

#[post("/login", data = "<login_form>")]
async fn login<'a>(cookies: &'a CookieJar<'_>, login_form: Json<Login>) -> rocket::response::Response<'a> {
    let valid = model::validate_user(
        login_form.username.clone(),
        (&login_form.password).as_ref(),
    ).await;

    if let Err(e) = &valid {
        let status = match e {
            LoginFailure::BadLogin => { rocket::http::Status::Forbidden }
            LoginFailure::SqlError(e) => {
                error!("{}", e);
                rocket::http::Status::InternalServerError
            }
        };

        return Response::build()
            .status(status)
            .finalize();
    }

    let token = model::jwt_generate(valid.unwrap());
    cookies.add(Cookie::new("auth", token));

    Response::new()
}

#[delete("/login")]
async fn logout<'a>(cookies: &'a CookieJar<'_>) -> Response<'a> {
    let mut resp = Response::new();
    let login_cook = cookies.get("auth");

    if login_cook.is_none() {
        resp.set_status(Status::Forbidden);
        return resp;
    }

    cookies.remove(Cookie::named("auth"));

    resp
}

#[post("/alias", data = "<alias_form>")]
async fn new_or_update_alias(user: Claims, alias_form: Json<AliasForm>) -> Response<'static> {
    let mut resp = Response::new();
    let dest = Url::parse(&alias_form.to);

    let dest = match dest {
        Ok(d) => {
            d.to_string()
        }
        Err(e) => {
            resp.set_status(Status::BadRequest);
            resp.set_header(ContentType::JSON);
            let body = json!({
                "message": "Invalid URL",
                "info": format!("{}", e),
            }).to_string();
            resp.set_sized_body(body.len(), Cursor::new(body));
            return resp;
        }
    };

    info!("Added alias {} to {} for user {}",
          &alias_form.from,
          &dest,
          &user.user
    );

    let d = dest.clone();
    let orig = alias_form.from.clone();

    let result = conn()
        .with_conn(move |c| {
            use ::alias::schema::aliases::dsl::*;
            diesel::replace_into(aliases)
                .values((creator.eq(&user.user_id),
                         alias.eq(&alias_form.from),
                         destination.eq(dest)))
                .execute(c)
        }).await;

    resp.set_header(ContentType::JSON);
    let body = match result {
        Ok(u) if u != 1 => {
            resp.set_status(Status::InternalServerError);
            json!({
                "message": "failed to add alias"
            })
        }
        Err(e) => {
            resp.set_status(Status::InternalServerError);
            error!("{}", e);
            json!({
                "message": "failed to add alias"
            })
        }
        _ => {
            resp.set_status(Status::Created);
            json!({
                "message": "Added alias.",
                "from": orig,
                "to": d
            })
        }
    }.to_string();
    resp.set_sized_body(body.len(), Cursor::new(body));

    resp
}

#[delete("/<alias>")]
async fn delete_alias(user: Claims, alias: String) -> Response<'static> {
    let a = Arc::new(alias);
    let qa = a.clone();
    let res: QueryResult<usize> = conn().with_conn(move |c| {
        use ::alias::schema::aliases::dsl::*;
        diesel::delete(aliases)
            .filter(creator.eq(user.user_id))
            .filter(alias.eq(&*qa))
            .execute(c)
    }).await;

    let mut resp = Response::new();

    resp.set_header(ContentType::JSON);

    match res {
        Ok(s) if s != 1 => {
            resp.set_status(Status::NotFound);
            let body = json!({
                "message": "No such alias"
            }).to_string();
            resp.set_sized_body(body.len(), Cursor::new(body));
        }
        Err(e) => {
            error!("{}", e);
            resp.set_status(Status::InternalServerError);
            let body = json!({
                "message": "Internal server error"
            }).to_string();
            resp.set_sized_body(body.len(), Cursor::new(body));
        }
        _ => {
            cache::evict_alias(a.to_string()).await;
            let body = json!({
                "message": "Successfully deleted alias",
                "alias": a.as_str()
            }).to_string();
            resp.set_sized_body(body.len(), Cursor::new(body));
        }
    };

    resp
}

#[get("/<alias>")]
async fn get_alias(alias: String) -> Response<'static> {
    let res_dest = cache::get_alias(alias.clone()).await;
    let mut resp = Response::new();
    resp.set_header(ContentType::JSON);

    match res_dest {
        Err(e) => {
            let
                body = match e {
                AliasSearchFailure::NoSuchAlias => {
                    resp.set_status(Status::NotFound);
                    json!({
                    "message": "No such alias",
                    "alias": alias
                })
                }
                AliasSearchFailure::Sql(e) => {
                    error!("{}", e);
                    resp.set_status(Status::InternalServerError);
                    json!({
                    "message": "An internal error occurred."
                })
                }
            }.to_string();

            resp.set_sized_body(body.len(), Cursor::new(body));
        },
        Ok(d) => {
            resp.set_status(Status::Found);
            let body = json!({
                "message": "redirected",
                "to": &d
            }).to_string();
            resp.set_raw_header("Location", d.clone());
            resp.set_sized_body(body.len(), Cursor::new(body));
        }
    }

    resp
}


embed_migrations!();

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    better_panic::install();

    let uname = Arg::new("name")
        .required(true)
        .takes_value(true)
        .index(1);

    let matches = clap::App::new("aliasd")
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .arg(Arg::new("config-file")
            .short('c')
            .takes_value(true)
            .default_value(".env")
            .about("Allows for alternate locations for a dotenv file"))
        .arg(Arg::new("verbose")
            .short('v')
            .multiple_occurrences(true)
            .about("Controls logging verbosity.")
            .hidden(false))
        .subcommand(App::new("run")
            .about("Starts the server.")
        )
        .subcommand(App::new("user")
            .about("Commands related to users.")
            .subcommand(
                App::new("add")
                    .about("Adds an approved user to the database.")
                    .arg(uname.clone())
            )
            .subcommand(
                App::new("del")
                    .about("Removes a user from the database, along with all their aliases.")
                    .arg(uname.clone())
            )
            .subcommand(
                App::new("list")
                    .about("Prints a list of registered users.")
            )
            .setting(AppSettings::SubcommandRequired)
        )
        .setting(AppSettings::SubcommandRequired)
        .get_matches();

    let env_file = matches.value_of_os("config-file")
        .expect("Need environment file location.");

    dotenv::from_filename(env_file).ok();

    let verbose = matches.occurrences_of("verbose");
    let log_level = log_level(verbose);
    let rocket_level = translate_level(log_level);

    let sub = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(log_level)
        .finish();

    tracing::subscriber::set_global_default(sub).unwrap();

    let db_path = db::db_path();
    let conn = db::establish_connection(db_path);
    db::init_service(conn);

    db::conn().with_conn(|c| {
        embedded_migrations::run(c)
    }).await?;

    match matches.subcommand().unwrap() {
        ("run", _) => {
            let mut cfg = Config::from(Config::figment());
            cfg.log_level = rocket_level;
            rocket::custom(cfg)
                .attach(SpaceHelmet::default())
                .mount("/", routes![get_alias, delete_alias, new_or_update_alias, logout, login])
                .launch()
                .await?;
        }
        ("user", m) => {
            match m.subcommand().unwrap() {
                ("list", _) => {}
                (x, m) => {
                    let user = m.value_of("name").unwrap();
                    match x {
                        "add" => {
                            users::add_user_interactive(user).await?;
                        }
                        "del" => {
                            users::del_user(user).await?;
                        }
                        _ => unreachable!()
                    }
                }
            }
        }
        _ => unreachable!()
    }


    Ok(())
}

fn log_level(i: u64) -> tracing::Level {
    match i {
        0 => tracing::Level::INFO,
        1 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE
    }
}

fn translate_level(tl: tracing::Level) -> rocket::logger::LogLevel {
    match tl {
        Level::DEBUG | Level::TRACE => rocket::logger::LogLevel::Debug,
        Level::INFO | Level::WARN => rocket::logger::LogLevel::Normal,
        Level::ERROR => rocket::logger::LogLevel::Critical
    }
}