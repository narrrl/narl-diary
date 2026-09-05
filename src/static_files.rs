use axum::{
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;

/// The built Svelte app. In debug builds rust-embed reads from disk, so a
/// `vite build --watch` next to `cargo run` is enough for iteration.
#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/web/dist"]
struct Assets;

pub async fn handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    match Assets::get(path) {
        Some(file) => serve(path, file),
        // Unknown paths are client-side routes; hand them the shell.
        None => match Assets::get("index.html") {
            Some(index) => serve("index.html", index),
            None => (
                StatusCode::NOT_FOUND,
                "frontend not built - run `bun run build` in web/",
            )
                .into_response(),
        },
    }
}

fn serve(path: &str, file: rust_embed::EmbeddedFile) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    // Vite fingerprints everything under /assets/, so it is safe to cache hard.
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (header::CONTENT_TYPE, mime.as_ref()),
            (header::CACHE_CONTROL, cache),
        ],
        file.data,
    )
        .into_response()
}
