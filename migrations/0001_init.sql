-- POSTS
create table if not exists posts (
  id uuid primary key,
  slug text not null unique,
  title text not null,
  markdown text not null,
  html text not null,
  tags jsonb not null default '[]'::jsonb,
  status text not null default 'draft',
  published_at timestamptz null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create index if not exists idx_posts_status_published_at on posts(status, published_at desc);

-- IMAGES (guardadas en DB)
create table if not exists images (
  id uuid primary key,
  filename text not null,
  mime_type text not null,
  bytes bytea not null,
  created_at timestamptz not null default now()
);
