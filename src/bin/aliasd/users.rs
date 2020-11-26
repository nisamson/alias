use alias::model::{UserCreateError};
use alias::db::conn;
use alias::pass::create_pass_hash;
use diesel::{RunQueryDsl, ExpressionMethods};

pub async fn add_user_interactive(user: impl AsRef<str>) -> Result<(), UserCreateError> {
    let pass = tokio::task::spawn_blocking(|| {
        let pass1 = rpassword::read_password_from_tty(Option::from("Please enter a password: ")).unwrap();
        let pass2 = rpassword::read_password_from_tty(Option::from("Re-enter the password: "))
            .unwrap();

        if pass1 != pass2 {
            eprintln!("Password mismatch.");
            std::process::exit(1);
        }

        if pass1.len() < 8 {
            eprintln!("Password too short. Must be at least 8 bytes.");
            std::process::exit(1);
        }

        pass1
    }).await.unwrap();

    let pass_hash = create_pass_hash(pass);

    let u = user.as_ref();
    let cu = u.to_string();

    alias::model::create_user(cu, pass_hash).await?;

    println!("Added user {}", u);

    Ok(())
}

pub async fn del_user(user: impl Into<String>) -> anyhow::Result<()> {
    let user = user.into();
    let us = user.clone();
    conn().with_conn(move |c| {
        use alias::schema::users::dsl::*;
        let num_affected = diesel::delete(users)
            .filter(username.eq(&us))
            .execute(c)?;

        if num_affected != 1 {
            eprintln!("Doesn't seem like there was a user by that name.");
            std::process::exit(1);
        }

        Result::<_, diesel::result::Error>::Ok(())

    }).await?;

    println!("Deleted user {}", user);
    Ok(())
}