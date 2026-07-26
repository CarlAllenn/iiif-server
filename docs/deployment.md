# Deployment recipes

The engine is deliberately narrow: stateless, no TLS, no auth, no
derivative cache. Each of those is one proven layer in front of it.

## Running

```bash
iiif-server serve ./images
```

```bash
iiif-server serve s3://bucket/prefix --endpoint https://objects.example.com
```

Credentials come from the environment (`AWS_ACCESS_KEY_ID`,
`AWS_SECRET_ACCESS_KEY`, or the platform's IMDS/IRSA/workload-identity
machinery — `object_store` owns that swamp). The only other knobs:

| Flag | Default | Meaning |
| --- | --- | --- |
| `--bind` | `127.0.0.1:6363` | listen address |
| `--public-base` | from Host header | scheme+authority used in `id`/`@id` and canonical links |
| `--max-width/--max-height` | 8192 | published and enforced size limits |
| `--max-area` | 33554432 (32 MP) | published and enforced area limit |
| `--workers` | CPU count | concurrent decode bound |
| `--queue-depth` | 64 | admitted waiters beyond the workers; overflow → 503 + Retry-After |
| `--endpoint` | — | S3-compatible endpoint URL |

Endpoints: `/iiif/3/…` (Image API 3.0), `/iiif/2/…` (Image API 2.1),
`/healthz`, `/metrics` (Prometheus text).

Before pointing the server at a collection, run the offline inspector —
it prints per-master serving advice with copy-paste fixes:

```bash
iiif-server check ./images
```

## TLS + caching: any CDN or reverse proxy

Derivatives are immutable per canonical URL and carry strong ETags, so
ordinary HTTP caching does all the work. nginx sketch:

```nginx
proxy_cache_path /var/cache/iiif keys_zone=iiif:64m max_size=20g inactive=30d;

server {
    listen 443 ssl;
    # ssl_certificate …; ssl_certificate_key …;

    location /iiif/ {
        proxy_pass http://127.0.0.1:6363;
        proxy_cache iiif;
        proxy_cache_valid 200 30d;
        proxy_cache_use_stale error timeout updating;
        proxy_cache_lock on;      # collapse concurrent misses per tile
    }
}
```

A CDN in front (any of them) works the same way: honor `Cache-Control`
and `ETag`, key on the full path. `503 + Retry-After` from the engine
means the decode pool is saturated — let it surface rather than retrying
instantly.

## Access control: forward-auth at the proxy

The engine serves whatever it is asked for; the proxy decides who asks.
nginx `auth_request` sketch:

```nginx
location /iiif/ {
    auth_request /_authz;
    proxy_pass http://127.0.0.1:6363;
}

location = /_authz {
    internal;
    proxy_pass http://auth-service/check;
    proxy_pass_request_body off;
    proxy_set_header X-Original-URI $request_uri;
}
```

Traefik ForwardAuth / Caddy `forward_auth` are equivalent. Per-image
policy belongs to the auth service, which sees the original URI
(identifier included).

## Systemd sketch

```ini
[Service]
ExecStart=/usr/local/bin/iiif-server serve /srv/images --bind 127.0.0.1:6363 \
    --public-base https://images.example.org
Restart=on-failure
DynamicUser=yes
ProtectSystem=strict
ReadOnlyPaths=/srv/images
NoNewPrivileges=yes
```

The binary is static (musl) and needs no shared libraries, no config
file, and no writable filesystem.
