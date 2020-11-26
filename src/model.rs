use crate::db::{conn, DBError};
use diesel::{RunQueryDsl, QueryDsl, ExpressionMethods, QueryResult, select};
use crate::schema::*;
use crate::model::LoginFailure::BadLogin;
use jsonwebtoken::{Header, EncodingKey, DecodingKey, Validation};
use crate::pass::SECRET_KEY;
use rocket::request::{FromRequest, Outcome};
use rocket::Request;
use rocket::http::Status;
use diesel::expression::exists::exists;

#[derive(Insertable)]
#[table_name = "users"]
pub struct NewUser<'a> {
    pub username: &'a str,
    pub hash: &'a str,
}

#[derive(Queryable)]
pub struct User {
    pub id: i32,
    pub username: String,
    pub hash: String,
}

#[derive(Debug, thiserror::Error)]
pub enum UserCreateError {
    #[error("A user by that name already exists.")]
    AlreadyExists,
    #[error("Something went wrong while running the query.")]
    SqlError(#[from] diesel::result::Error),
}

pub async fn create_user(u: String, h: String) -> Result<User, UserCreateError> {
    use crate::schema::users::dsl::*;

    conn().with_conn(move |c| {
        diesel::insert_into(users)
            .values(NewUser { username: &u, hash: &h })
            .execute(c)
            .map_err(|e| {
                if e.is_uniqueness_violation() {
                    UserCreateError::AlreadyExists
                } else {
                    e.into()
                }
            })?;

        let user: User = users.filter(username.eq(u)).first(c)?;
        Ok(user)
    }).await
}

#[derive(Debug, thiserror::Error)]
pub enum LoginFailure {
    #[error("Invalid login info.")]
    BadLogin,
    #[error("Something went wrong while running the query.")]
    SqlError(#[from] diesel::result::Error),
}

pub async fn validate_user(u: String, pass: &[u8]) -> Result<User, LoginFailure> {
    let user = conn().with_conn(move |c| {
        use crate::schema::users::dsl::*;
        let user: User = users
            .filter(username.eq(u))
            .first(c)
            .map_err(|e| if matches!(&e, diesel::result::Error::NotFound) {
                LoginFailure::BadLogin
            } else {
                e.into()
            })?;

        Result::<_, LoginFailure>::Ok(user)
    }).await?;

    argon2::verify_encoded_ext(&user.hash, pass, (&*crate::pass::SECRET_KEY).as_ref(), &[])
        .map_err(|_| BadLogin)
        .and_then(|b| if b { Ok(user) } else { Err(BadLogin) })
}

pub async fn user_exists(u: String) -> QueryResult<bool> {
    conn().with_conn(move |c| {
        use crate::schema::users::dsl::*;
        select(exists(users.filter(username.eq(u))))
            .get_result::<bool>(c)
    }).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub user: String,
    pub user_id: i32,
    pub iat: i64,
    pub exp: i64,
}

#[rocket::async_trait]
impl<'a, 'r> FromRequest<'a, 'r> for Claims {
    type Error = ();

    async fn from_request(request: &'a Request<'r>) -> Outcome<Self, Self::Error> {
        let cookies = request.cookies();
        let jwt = cookies.get("auth")
            .map(|c| c.value())
            .ok_or(())
            .and_then(|s| jsonwebtoken::decode::<Claims>(s, &DecodingKey::from_secret(SECRET_KEY.as_bytes()), &Validation::default()).map_err(|_| ()))
            ;

        if let Err(_) = jwt {
            return Outcome::Failure((Status::Forbidden, ()));
        }

        let token = jwt.unwrap().claims;
        let user_exists = user_exists(token.user.clone()).await;

        if let Err(e) = &user_exists {
            error!("{}", e);
            return Outcome::Failure((Status::InternalServerError, ()));
        }

        let e = user_exists.unwrap();
        if e {
            Outcome::Success(token)
        } else {
            Outcome::Failure((Status::Forbidden, ()))
        }
    }
}

pub fn jwt_generate(user: User) -> String {
    let now = chrono::Local::now();
    let now_plus_week = now + chrono::Duration::weeks(1);

    let payload = Claims {
        user: user.username,
        user_id: user.id,
        iat: now.timestamp(),
        exp: now_plus_week.timestamp(),
    };

    jsonwebtoken::encode(&Header::default(), &payload, &EncodingKey::from_secret(SECRET_KEY.as_bytes())).unwrap()
}

#[derive(FromForm, Serialize, Deserialize)]
pub struct Login {
    pub username: String,
    pub password: String,
}

#[derive(FromForm, Serialize, Deserialize)]
pub struct AliasForm {
    pub from: String,
    pub to: String,
}

