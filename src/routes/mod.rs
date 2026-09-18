pub mod backup;
pub mod boards;
pub mod nodes;
pub mod export;
pub mod import;
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
        .route("/export", get(export::export))
        .route("/backup", get(backup::status).post(backup::run))
        .route("/spaces", get(nodes::spaces).post(nodes::create_space))
        .route("/spaces/{id}/board", get(boards::board))
        .route("/spaces/{id}/lists", post(boards::create_list))
        .route("/lists/{id}", put(boards::rename_list).delete(boards::remove_list))
        .route("/lists/{id}/cards", post(boards::create_card))
        .route("/cards/{id}", put(boards::update_card).delete(boards::remove_card))
        .route("/cards/{id}/move", post(boards::move_card))
        .route("/nodes", get(nodes::list).post(nodes::create))
        .route("/resolve", get(nodes::resolve))
        .route("/nodes/{id}", get(nodes::get))
        .route("/nodes/{id}", put(nodes::update))
        .route("/nodes/{id}", delete(nodes::remove))
        .route("/nodes/{id}/move", post(nodes::move_node))
        .route("/nodes/{id}/import", post(import::import))
        .route("/nodes/{id}/share", post(share::enable))
        .route("/nodes/{id}/share", delete(share::disable))
        .route("/media", post(media::upload).get(media::list))
        .route("/media/{id}", get(media::serve).delete(media::remove))
        .route("/share/{token}/{key}", get(share::read))
        .route("/share/{token}/{key}/media/{id}", get(share::serve_media))
}
