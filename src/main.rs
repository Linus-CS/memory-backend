use std::env;

use memory_backend::memory::{MemoryStore, Store};
use memory_backend::queries::{CreateQuery, JoinQuery, PickQuery};
use memory_backend::reject::handle_rejection;
use tokio::sync::RwLock;
use warp::Filter;

use crate::handler::*;

mod handler;

#[tokio::main]
async fn main() {
    let key = env::var("MASTER_KEY").expect("No MASTER_KEY set");

    let cors = warp::cors()
        .allow_any_origin()
        .allow_credentials(true)
        .allow_headers(vec![
            "User-Agent",
            "Sec-Fetch-Mode",
            "Referer",
            "Origin",
            "Access-Contro-Allow-Origin",
            "Access-Control-Request-Method",
            "Access-Control-Request-Headers",
            "Content-Type",
            "Authorization",
        ])
        .allow_methods(vec!["GET", "POST", "PUT", "DELETE", "OPTIONS"]);

    let store = Store::new(RwLock::new(MemoryStore {
        game: None,
        master_key: key.clone(),
    }));
    let store = warp::any().map(move || store.clone());

    let ping_route = warp::get()
        .and(warp::cookie::optional("memory_token"))
        .and(warp::path("ping"))
        .and(warp::path::end())
        .and(store.clone())
        .and_then(ping);

    let key_route = warp::get()
        .and(warp::path("key"))
        .and(warp::query::raw())
        .and(warp::path::end())
        .and(store.clone())
        .and_then(check_key);

    let create_route = warp::post()
        .and(warp::cookie("master_key"))
        .and(warp::path("create"))
        .and(warp::query::<CreateQuery>())
        .and(warp::path::end())
        .and(store.clone())
        .and_then(create);

    let delete_route = warp::post()
        .and(warp::cookie("master_key"))
        .and(warp::path("delete"))
        .and(warp::path::end())
        .and(store.clone())
        .and_then(delete);

    let join_route = warp::post()
        .and(warp::path("join"))
        .and(warp::query::<JoinQuery>())
        .and(warp::path::end())
        .and(store.clone())
        .and_then(join);

    let game_route = warp::get()
        .and(warp::path("game"))
        .and(warp::cookie("memory_token"))
        .and(warp::path::end())
        .and(store.clone())
        .and_then(game_message);

    let ready_route = warp::post()
        .and(warp::cookie("memory_token"))
        .and(warp::path("ready"))
        .and(warp::path::end())
        .and(store.clone())
        .and_then(ready);

    let pick_card_route = warp::post()
        .and(warp::cookie("memory_token"))
        .and(warp::path("pick_card"))
        .and(warp::query::<PickQuery>())
        .and(warp::path::end())
        .and(store.clone())
        .and_then(pick_card);

    let image_route = warp::path("img").and(warp::fs::dir("images"));

    let routes = ping_route
        .or(key_route)
        .or(create_route)
        .or(delete_route)
        .or(join_route)
        .or(game_route)
        .or(ready_route)
        .or(pick_card_route)
        .or(image_route)
        .with(cors)
        .recover(handle_rejection);

    let port: String = env::var("PORT").unwrap_or("8080".to_owned());
    let port = port.parse::<u16>().expect("PORT is not a valid number");

    println!("Listening on port {port}");
    warp::serve(routes).run(([0, 0, 0, 0], port)).await;
}
