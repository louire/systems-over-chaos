use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Form, Json, Router,
};

use tower_http::services::ServeDir;

use askama::{Html as AskamaHtml, MarkupDisplay, Template};
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use pulldown_cmark::{html, Options, Parser};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::{env, net::SocketAddr};
use uuid::Uuid;
use dotenvy::dotenv;

/* ------------------------ State ------------------------ */

#[derive(Clone)]
struct AppState {
    db: PgPool,
    admin_user: String,
    admin_pass: String,
}

/* ------------------------ Main ------------------------ */

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();

    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3000);

    let database_url =
        env::var("DATABASE_URL").expect("DATABASE_URL is required (Railway Postgres provides it)");
    let admin_user = env::var("ADMIN_USER").unwrap_or_else(|_| "admin".to_string());
    let admin_pass = env::var("ADMIN_PASS").expect("ADMIN_PASS is required");

    let db = PgPool::connect(&database_url).await?;

    run_migrations(&db).await?;

    let state = AppState {
        db,
        admin_user,
        admin_pass,
    };

    let app = Router::new()
        .nest_service("/static", ServeDir::new("static"))
        // Public
        .route("/", get(index))
        .route("/blog/{slug}", get(show_post))
        .route("/img/{id}", get(get_image))
        // Admin
        .route("/admin", get(admin_home))
        .route("/admin/new", get(admin_new))
        .route("/admin/preview", post(admin_preview))
        .route("/admin/create", post(admin_create))
        .route("/admin/upload-image", post(admin_upload_image))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("Systems Over Chaos listening on http://{addr}");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;
    Ok(())
}

async fn run_migrations(db: &PgPool) -> anyhow::Result<()> {
    let sql = include_str!("../migrations/0001_init.sql");

    for stmt in sql.split(';') {
        let s = stmt.trim();
        if s.is_empty() {
            continue;
        }
        sqlx::query(&format!("{s};")).execute(db).await?;
    }
    Ok(())
}

/* ------------------------ Helpers ------------------------ */

fn check_basic_auth(headers: &HeaderMap, user: &str, pass: &str) -> bool {
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let Some(auth) = auth else { return false; };

    let Some(encoded) = auth.strip_prefix("Basic ") else {
        return false;
    };

    let Ok(decoded) = general_purpose::STANDARD.decode(encoded) else {
        return false;
    };

    let Ok(decoded) = String::from_utf8(decoded) else {
        return false;
    };

    decoded == format!("{user}:{pass}")
}

fn md_to_sanitized_html(md: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(md, options);
    let mut raw_html = String::new();
    html::push_html(&mut raw_html, parser);

    ammonia::Builder::default()
        .add_tags([
            "img", "h1", "h2", "h3", "h4", "pre", "code", "blockquote",
            "hr", "br", "table", "thead", "tbody", "tr", "th", "td",
        ])
        .add_generic_attributes([
            "class", "id"
        ])
        .add_tag_attributes("img", ["src", "alt", "title", "loading"])
        .add_tag_attributes("a", ["href", "title", "target", "rel"])
        .add_tag_attributes("code", ["class"])
        .add_allowed_classes("code", ["language-rust", "language-js", "language-json", "language-bash"])
        .url_relative(ammonia::UrlRelative::PassThrough) // permite /img/...
        .clean(&raw_html)
        .to_string()
}


/* ------------------------ Templates ------------------------ */

#[derive(Template)]
#[template(path = "public_index.html")]
struct PublicIndexTpl {
    posts: Vec<PostCard>,
}

#[derive(Template)]
#[template(path = "public_post.html", escape = "none")]
struct PublicPostTpl {
    title: String,
    published_at: String,
    tags: Vec<String>,
    html: String,
}

#[derive(Template)]
#[template(path = "admin_home.html")]
struct AdminHomeTpl {
    base_url: String,
}

#[derive(Template)]
#[template(path = "admin_new.html")]
struct AdminNewTpl {}

/* ------------------------ Models ------------------------ */

#[derive(Clone)]
struct PostCard {
    title: String,
    slug: String,
    published_at: String,
}

/* ------------------------ Public Handlers ------------------------ */

async fn index(State(st): State<AppState>) -> impl IntoResponse {
    let rows = sqlx::query(
        r#"
        select title, slug, published_at
        from posts
        where status = 'published'
        order by published_at desc nulls last, created_at desc
        limit 50
        "#,
    )
    .fetch_all(&st.db)
    .await
    .unwrap_or_default();

    let posts: Vec<PostCard> = rows
        .into_iter()
        .map(|r| {
            let published_at: Option<DateTime<Utc>> = r.try_get("published_at").ok();
            PostCard {
                title: r.get("title"),
                slug: r.get("slug"),
                published_at: published_at
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| "-".to_string()),
            }
        })
        .collect();

    Html(PublicIndexTpl { posts }.render().unwrap())
}

async fn show_post(State(st): State<AppState>, Path(slug): Path<String>) -> impl IntoResponse {
    let row = sqlx::query(
        r#"
        select title, html, tags, published_at
        from posts
        where slug = $1 and status = 'published'
        "#,
    )
    .bind(&slug)
    .fetch_optional(&st.db)
    .await
    .unwrap();

    let Some(row) = row else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };

    let tags_json: serde_json::Value = row.get("tags");
    let tags: Vec<String> = tags_json
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();

    let published_at: Option<DateTime<Utc>> = row.try_get("published_at").ok();
    let published_at = published_at
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "-".to_string());

    let html_str: String = row.get("html");

    let tpl = PublicPostTpl {
        title: row.get("title"),
        html: html_str,
        tags,
        published_at,
    };

    Html(tpl.render().unwrap()).into_response()
}

async fn get_image(State(st): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let Ok(uuid) = Uuid::parse_str(&id) else {
        return (StatusCode::BAD_REQUEST, "Bad image id").into_response();
    };

    let row = sqlx::query("select mime_type, bytes from images where id=$1")
        .bind(uuid)
        .fetch_optional(&st.db)
        .await
        .unwrap();

    let Some(row) = row else {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };

    let mime: String = row.get("mime_type");
    let bytes: Vec<u8> = row.get("bytes");

    ([(header::CONTENT_TYPE, mime)], bytes).into_response()
}

/* ------------------------ Admin Handlers ------------------------ */

async fn admin_home(State(st): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !check_basic_auth(&headers, &st.admin_user, &st.admin_pass) {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, r#"Basic realm="Systems Over Chaos Admin""#)],
            "Unauthorized",
        )
            .into_response();
    }

    let base_url = env::var("BASE_URL").unwrap_or_else(|_| "".to_string());
    Html(AdminHomeTpl { base_url }.render().unwrap()).into_response()
}

async fn admin_new(State(st): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !check_basic_auth(&headers, &st.admin_user, &st.admin_pass) {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, r#"Basic realm="Systems Over Chaos Admin""#)],
            "Unauthorized",
        )
            .into_response();
    }

    Html(AdminNewTpl {}.render().unwrap()).into_response()
}

#[derive(Deserialize)]
struct PreviewReq {
    markdown: String,
}

#[derive(Serialize)]
struct PreviewRes {
    html: String,
}

async fn admin_preview(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PreviewReq>,
) -> impl IntoResponse {
    if !check_basic_auth(&headers, &st.admin_user, &st.admin_pass) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let html = md_to_sanitized_html(&req.markdown);
    Json(PreviewRes { html }).into_response()
}

#[derive(Deserialize)]
struct CreatePostForm {
    title: String,
    slug: String,
    tags: String,
    status: String,
    markdown: String,
}

async fn admin_create(
    State(st): State<AppState>,
    headers: HeaderMap,
    Form(f): Form<CreatePostForm>,
) -> impl IntoResponse {
    if !check_basic_auth(&headers, &st.admin_user, &st.admin_pass) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let slug = f.slug.trim().to_lowercase();
    if slug.is_empty() {
        return (StatusCode::BAD_REQUEST, "slug required").into_response();
    }

    let tags = f
        .tags
        .split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| serde_json::Value::String(t.to_string()))
        .collect::<Vec<_>>();

    let html = md_to_sanitized_html(&f.markdown);
    let id = Uuid::new_v4();

    let status = if f.status == "published" {
        "published"
    } else {
        "draft"
    };

    let published_at = if status == "published" { Some(Utc::now()) } else { None };

    let res = sqlx::query(
        r#"
        insert into posts (id, slug, title, markdown, html, tags, status, published_at)
        values ($1,$2,$3,$4,$5,$6,$7,$8)
        "#,
    )
    .bind(id)
    .bind(&slug)
    .bind(&f.title)
    .bind(&f.markdown)
    .bind(&html)
    .bind(serde_json::Value::Array(tags))
    .bind(status)
    .bind(published_at)
    .execute(&st.db)
    .await;

    if let Err(e) = res {
        return (StatusCode::BAD_REQUEST, format!("db error: {e}")).into_response();
    }

    if status == "published" {
        return (
            StatusCode::SEE_OTHER,
            [(header::LOCATION, format!("/blog/{slug}"))],
            "",
        )
            .into_response();
    }

    (StatusCode::SEE_OTHER, [(header::LOCATION, "/admin")], "").into_response()
}

/* ------------------------ Image Upload ------------------------ */

use axum::extract::Multipart;

async fn admin_upload_image(
    State(st): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if !check_basic_auth(&headers, &st.admin_user, &st.admin_pass) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    while let Ok(Some(field)) = multipart.next_field().await {
        let field_name = field.name().unwrap_or("").to_string();
        if field_name != "file" {
            continue;
        }

        let filename = field.file_name().unwrap_or("upload.bin").to_string();

        let mime = field
            .content_type()
            .map(|ct| ct.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let data = match field.bytes().await {
            Ok(b) => b,
            Err(_) => return (StatusCode::BAD_REQUEST, "Failed to read bytes").into_response(),
        };

        if data.len() > 5 * 1024 * 1024 {
            return (StatusCode::BAD_REQUEST, "image too large (max 5MB)").into_response();
        }

        let id = Uuid::new_v4();
        let res = sqlx::query(
            "insert into images (id, filename, mime_type, bytes) values ($1,$2,$3,$4)",
        )
        .bind(id)
        .bind(filename)
        .bind(mime)
        .bind(data.as_ref())
        .execute(&st.db)
        .await;

        if let Err(e) = res {
            return (StatusCode::BAD_REQUEST, format!("db error: {e}")).into_response();
        }

        let url = format!("/img/{id}");
        return (StatusCode::OK, url).into_response();
    }

    (StatusCode::BAD_REQUEST, "missing file").into_response()
}
