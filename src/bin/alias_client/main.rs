use clap::{App, Arg, AppSettings};
use reqwest::redirect::Policy;
use url::Url;
use alias::model::{Login, AliasForm};

#[macro_use]
extern crate tracing;


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    better_panic::install();
    dotenv::dotenv().ok();

    let alias_arg = Arg::new("alias")
        .index(1)
        .takes_value(true)
        .required(true)
        .about("An alias for redirection with an aliasd server.");

    let matches = App::new("alias-client")
        .arg(Arg::new("verbosity")
            .takes_value(false)
            .multiple_occurrences(true)
            .short('v')
            .about("Controls logging verbosity"))
        .arg(Arg::new("username")
            .about("The username for use when connecting to the server")
            .env("ALIAS_USERNAME")
            .takes_value(true)
            .required(false)
            .default_value("")
            .short('u'))
        .arg(Arg::new("server")
            .about("The alias server to connect to")
            .env("ALIAS_URL")
            .required(false)
            .takes_value(true)
            .short('s')
        )
        .subcommand(
            App::new("check")
                .about("Retrieves the current value of an alias from the alias server.")
                .arg(alias_arg.clone()))
        .subcommand(
            App::new("del")
                .about("Deletes the specified alias from the alias server")
                .arg(alias_arg.clone()))
        .subcommand(
            App::new("add")
                .about("Creates or updates an alias.")
                .arg(alias_arg)
                .arg(
                    Arg::new("destination")
                        .about("The destination for redirection when the alias is used.")
                        .required(true)
                        .index(2)
                )
        )
        .setting(AppSettings::SubcommandRequired)
        .get_matches();

    let username = matches.value_of_t::<String>("username")?;
    let verbosity = matches.occurrences_of("verbosity");

    let log_level = log_level(verbosity);
    let sub = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(log_level)
        .finish();

    tracing::subscriber::set_global_default(sub).unwrap();

    let server_url = matches.value_of_t::<Url>("server")?;

    match matches.subcommand().unwrap() {
        ("check", m) => {
            let alias = m.value_of_t::<String>("alias")?;
            let url = server_url.join(&alias)?;

            let client = reqwest::ClientBuilder::default()
                .redirect(Policy::custom(|a| {
                    a.stop()
                }))
                .build()?;

            let resp = client.get(url)
                .send()
                .await?;

            if !resp.status().is_redirection() {
                warn!("Didn't get a redirect: {}", resp.status());
                std::process::exit(1);
            } else {
                let dest = resp.headers()
                    .get("Location")
                    .expect("Got a redirect but no location.");

                let dest = dest.to_str().expect("Got URL, but it wasn't valid UTF-8");
                info!("Alias {} redirects to {}", &alias, dest);
            }
        }
        (op, m) => {
            let alias = m.value_of_t::<String>("alias")?;
            let dest = if op == "add" {
                m.value_of_t::<Url>("destination")?
            } else {
                Url::parse("http://localhost").unwrap()
            };

            let pass = std::env::var("ALIAS_PASSWORD")
                .unwrap_or_else(|_| {
                    rpassword::read_password_from_tty(Some("Please enter your password: ")).unwrap()
                });

            let client = reqwest::ClientBuilder::default()
                .cookie_store(true)
                .build()?;

            let login = server_url.join("login")?;

            info!("Logging in as {}...", &username);
            let resp = client.post(login)
                .json(&Login { username, password: pass })
                .send()
                .await?;

            if !resp.status().is_success() {
                error!("Failed to login: {}", resp.status());
                std::process::exit(1);
            }
            info!("Logged in.");

            if op == "add" {
                info!("Adding alias from {} to {}", &alias, &dest);
                let resp = client.post(server_url.join("alias")?)
                    .json(&AliasForm { from: alias, to: dest.into_string() })
                    .send()
                    .await?;
                if !resp.status().is_success() {
                    let body = resp.text().await?;
                    error!("Couldn't add alias: {}", body);
                    std::process::exit(1);
                }
                info!("Added alias.");
            } else {
                info!("Deleting alias {}", &alias);
                let resp = client.delete(server_url.join(&alias)?)
                    .send()
                    .await?;
                if !resp.status().is_success() {
                    let body = resp.text().await?;
                    error!("Couldn't delete alias: {}", body);
                    std::process::exit(1);
                }
                info!("Deleted alias.");
            }
        }
    };

    Ok(())
}

fn log_level(i: u64) -> tracing::Level {
    match i {
        0 => tracing::Level::INFO,
        1 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE
    }
}