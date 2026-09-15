# Systems Over Chaos

Small blog with a built-in admin panel. Write posts in Markdown, publish/unpublish them, upload images — all stored in Postgres, no external CMS.

## Stack

- **Rust** + **Axum** for the web server and routing
- **Askama** for HTML templates (compiled at build time)
- **Postgres** via **sqlx**, migrations run automatically on startup
- **pulldown-cmark** to turn Markdown into HTML, sanitized with **ammonia** before it's stored
- Images are uploaded and stored directly in Postgres (no S3/disk)
- Admin routes are protected with plain HTTP Basic Auth

## Running locally

You need a Postgres database (local or remote, e.g. a free Railway instance).

```bash
cp .env.example .env   # fill in your values
cargo run
```

The server runs migrations on boot, so the first run sets up the `posts` and `images` tables for you.

## Environment variables

| Variable | Required | Default | Notes |
|---|---|---|---|
| `DATABASE_URL` | yes | — | Postgres connection string |
| `ADMIN_PASS` | yes | — | Password for `/admin` |
| `ADMIN_USER` | no | `admin` | Username for `/admin` |
| `PORT` | no | `3000` | |
| `BASE_URL` | no | `""` | Public URL of the site, if you need it in templates |

## Deploying

No Dockerfile needed — this deploys straight to Railway (or anything using Nixpacks): push the repo, add a Postgres plugin, set the env vars above, done. `DATABASE_URL` comes for free from Railway's Postgres addon.

## Project layout

```
src/main.rs        all the routes/handlers (public blog + admin)
templates/         Askama templates (.html)
static/            CSS, served as-is
migrations/         SQL run automatically on startup
```

## Main dependencies

- `axum` — web framework
- `tokio` — async runtime
- `sqlx` — Postgres client
- `askama` — templates
- `pulldown-cmark` — Markdown → HTML
- `ammonia` — HTML sanitizer (runs on every post before it's saved)
- `uuid`, `chrono` — ids and dates
- `dotenvy` — loads `.env` in development
