use std::convert::Infallible;

use rand::{thread_rng, Rng};
use tokio_stream::wrappers::ReceiverStream;
use warp::{reply::Json, sse::Event, Rejection, Reply};

use memory_backend::memory::{GameState, Memory, Player, Store};
use memory_backend::queries::{CreateQuery, JoinQuery, PickQuery};
use memory_backend::reject::{AlreadyExists, InvalidMasterKey, InvalidToken, NoGameExists};

pub async fn ping(query: Option<String>, store: Store) -> Result<impl Reply, Rejection> {
    let lock = store.read().await;
    if lock.game.is_none() {
        return Err(warp::reject::custom(NoGameExists));
    }

    let reply = warp::reply::json(&lock.game.as_ref().unwrap().id);
    if let Some(value) = query {
        if lock.game.as_ref().unwrap().players.get(&value).is_none() {
            println!("Removed token: {}", value);
            return Ok(warp::reply::with_status(
                reply,
                warp::http::StatusCode::GONE,
            ));
        }
    }

    Ok(warp::reply::with_status(reply, warp::http::StatusCode::OK))
}

pub async fn check_key(
    key: String,
    referer: String,
    store: Store,
) -> Result<impl Reply, Rejection> {
    let lock = store.read().await;
    if lock.master_key == key {
        let reply = warp::reply::reply();
        let reply = warp::reply::with_header(reply, "Access-Control-Allow-Origin", referer);
        let reply = warp::reply::with_header(reply, "Access-Control-Allow-Credentials", "true");
        Ok(warp::reply::with_header(
            reply,
            "Set-Cookie",
            format!(
                "master_key={}; max-age=31536000; SameSite=None; Secure; HttpOnly",
                key
            ),
        ))
    } else {
        Err(warp::reject::custom(InvalidMasterKey))
    }
}

pub async fn create(
    master_key: String,
    query: CreateQuery,
    store: Store,
) -> Result<Json, Rejection> {
    let mut lock = store.write().await;

    if master_key == lock.master_key {
        let new_id = query.id;
        if lock.game.is_some() {
            return Err(warp::reject::custom(AlreadyExists));
        }
        lock.game = Some(Memory::new(new_id.clone()));
        println!("Created game with id: {}", new_id);
        Ok(warp::reply::json(&"Success!"))
    } else {
        Err(warp::reject::custom(InvalidMasterKey))
    }
}

pub async fn join(query: JoinQuery, store: Store) -> Result<impl Reply, Rejection> {
    let token: String = thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(30)
        .map(char::from)
        .collect();

    let mut lock = store.write().await;

    lock.game
        .as_mut()
        .unwrap()
        .players
        .insert(token.clone(), Player::new(query.name.clone()));

    println!("{} joined and got the token: {}", query.name, token);

    let reply = warp::reply::with_header(
        warp::reply::json(&"Success"),
        "Access-Control-Allow-Credentials",
        "true",
    );

    Ok(warp::reply::with_header(
        reply,
        "set-cookie",
        format!(
            "memory_token={}; Path=/; SameSite=Strict; Max-Age=1209600; HttpOnly",
            token
        ),
    ))
}

pub async fn game_message(token: String, store: Store) -> Result<impl Reply, Rejection> {
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(2);
    let receiverStream = ReceiverStream::new(receiver);
    let stream = warp::sse::keep_alive().stream(receiverStream);
    Ok(warp::sse::reply(stream))
}

pub async fn pick_card(token: String, query: PickQuery, store: Store) -> Result<Json, Rejection> {
    let mut lock = store.write().await;
    let game = lock.game.as_mut().unwrap();
    let player = game.players.get_mut(&token);
    if player.is_none() {
        return Err(warp::reject());
    }
    let player = player.unwrap();
    if !player.turn {
        return Err(warp::reject());
    }

    // TODO: Check if card is valid
    Ok(warp::reply::json(&"Success"))
}

pub async fn ready(token: String, store: Store) -> Result<Json, Rejection> {
    let mut lock = store.write().await;
    let game = lock.game.as_mut().unwrap();
    let player = game.players.get_mut(&token);
    if player.is_none() {
        return Err(warp::reject::custom(InvalidToken));
    }
    let player = player.unwrap();
    player.ready = true;
    for (_, player) in game.players.iter() {
        if !player.ready {
            return Ok(warp::reply::json(&"Success"));
        }
    }
    game.state = GameState::Running;
    Ok(warp::reply::json(&"Started"))
}
