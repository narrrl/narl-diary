pub mod entries;
pub mod export;
pub mod media;
pub mod session;
pub mod share;

use axum::{
    routing::{delete, get, post, put},
    Router,
};

use crate::state::AppState;

pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/me", get(session::me))
        .route("/login", post(session::login))
        .route("/logout", post(session::logout))
        .route("/entries", get(entries::list).post(entries::create))
        .route("/export", get(export::export))
        .route("/entries/{id}", get(entries::get))
        .route("/entries/{id}", put(entries::update))
        .route("/entries/{id}", delete(entries::remove))
        .route("/entries/{id}/share", post(share::enable))
        .route("/entries/{id}/share", delete(share::disable))
        .route("/media", post(media::upload).get(media::list))
        .route("/media/{id}", get(media::serve).delete(media::remove))
        .route("/share/{token}", get(share::read))
        .route("/share/{token}/media/{id}", get(share::serve_media))
}
