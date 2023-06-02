use std::convert::Infallible;

use memory_backend::reply::{InitResponse, LeaderboardResponse};
use memory_backend::sse_utils::{broadcast_sse, send_sse};
use tokio::sync::RwLockWriteGuard;
use tokio_stream::wrappers::ReceiverStream;
use warp::reply::{WithHeader, WithStatus};
use warp::{reply::Json, sse::Event, Rejection, Reply};

use memory_backend::memory::{GameState, Memory, MemoryStore, Player, Store};
use memory_backend::queries::{CreateQuery, JoinQuery, PickQuery};
use memory_backend::reject::{
    AlreadyExists, AlreadyRunning, InvalidMasterKey, InvalidToken, NoGameExists, NotYetRunning,
    NotYourTurn,
};

pub async fn ping(query: Option<String>, store: Store) -> Result<impl Reply, Rejection> {
    let lock = store.read().await;
    if lock.game.is_none() {
        return Err(warp::reject::custom(NoGameExists));
    }

    let reply = warp::reply::json(&lock.game.as_ref().unwrap().id);
    if let Some(token) = query {
        if lock.game.as_ref().unwrap().players.get(&token).is_none() {
            return remove_cookie_response("memory_token", reply);
        }
    }

    let reply = warp::reply::with_status(reply, warp::http::StatusCode::OK);
    Ok(warp::reply::with_header(reply, "", ""))
}

pub async fn check_key(key: String, store: Store) -> Result<impl Reply, Rejection> {
    let lock = store.read().await;
    if lock.master_key == key {
        set_cookie_reponse("master_key", key)
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
        create_new_game(&mut lock, query.id)
    } else {
        Err(warp::reject::custom(InvalidMasterKey))
    }
}

pub async fn delete(master_key: String, store: Store) -> Result<Json, Rejection> {
    let mut lock = store.write().await;

    if master_key == lock.master_key {
        lock.game = None;
        print!("Game deleted.");
        Ok(warp::reply::json(&"Game deleted"))
    } else {
        Err(warp::reject::custom(InvalidMasterKey))
    }
}

pub async fn join(query: JoinQuery, store: Store) -> Result<impl Reply, Rejection> {
    let mut lock = store.write().await;
    let game = lock.game.as_mut().unwrap();

    match game.state {
        GameState::Lobby => (),
        _ => return Err(warp::reject::custom(AlreadyRunning)),
    }
    if let Ok(token) = game.add_new_player(query.name) {
        update_leaderboard(game.players.values().collect()).await;
        set_cookie_reponse("memory_token", token)
    } else {
        Err(warp::reject::custom(AlreadyExists))
    }
}

pub async fn game_message(token: String, store: Store) -> Result<impl Reply, Rejection> {
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(2);

    let mut lock = store.write().await;
    let game = lock.game.as_mut().unwrap();

    let player = game.players.get_mut(&token).unwrap();
    let ready = player.ready.clone();
    player.sender = Some(sender.clone());

    let receiver_stream = ReceiverStream::new(receiver);
    let stream = warp::sse::keep_alive().stream(receiver_stream);

    send_state(&game.get_state(ready), &sender).await;

    Ok(warp::sse::reply(stream))
}

pub async fn send_state(
    res: &InitResponse,
    sender: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
) {
    send_sse("state", res, Some(sender)).await;
}

pub async fn pick_card(token: String, query: PickQuery, store: Store) -> Result<Json, Rejection> {
    let mut lock = store.write().await;
    let game = lock.game.as_mut().unwrap();

    match game.state {
        GameState::Running => (),
        _ => return Err(warp::reject::custom(NotYetRunning)),
    }

    if let Some(player) = game.players.get(&token) {
        if !player.turn {
            return Err(warp::reject::custom(NotYourTurn));
        }
    } else {
        return Err(warp::reject::custom(InvalidToken));
    }

    let reply = game.pick_card(query.card, token).await;
    update_leaderboard(game.players.values().collect()).await;
    reply
}

pub async fn ready(token: String, store: Store) -> Result<Json, Rejection> {
    let mut lock = store.write().await;
    let game = lock.game.as_mut().unwrap();

    if let Some(player) = game.players.get_mut(&token) {
        player.ready = true;
        println!("{} is ready", player.name);
    } else {
        return Err(warp::reject::custom(InvalidToken));
    }

    for (_, player) in game.players.iter() {
        if !player.ready {
            update_leaderboard(game.players.values().collect()).await;
            return Ok(warp::reply::json(&"Success"));
        }
    }

    game.start().await;
    update_leaderboard(game.players.values().collect()).await;
    Ok(warp::reply::json(&"Started"))
}

fn set_cookie_reponse(key: &str, value: String) -> Result<WithHeader<impl Reply>, Rejection> {
    Ok(warp::reply::with_header(
        warp::reply(),
        "Set-Cookie",
        format!(
            "{}={}; Path=/; Max-Age=31536000; SameSite=None; Secure; HttpOnly",
            key, value,
        ),
    ))
}

fn remove_cookie_response(
    key: &str,
    reply: Json,
) -> Result<WithHeader<WithStatus<Json>>, Rejection> {
    println!("Removed token: {}", key);
    let reply = warp::reply::with_status(reply, warp::http::StatusCode::GONE);
    return Ok(warp::reply::with_header(
        reply,
        "Set-Cookie",
        format!("{}=0; Max-Age=0; SameSite=None; Secure; HttpOnly", key),
    ));
}

fn create_new_game(
    lock: &mut RwLockWriteGuard<MemoryStore>,
    id: String,
) -> Result<Json, Rejection> {
    if lock.game.is_some() {
        return Err(warp::reject::custom(AlreadyExists));
    }
    lock.game = Some(Memory::new(id.clone()));
    println!("Created game with id: {}", id);
    Ok(warp::reply::json(&"Success!"))
}

async fn update_leaderboard(players: Vec<&Player>) {
    let res = LeaderboardResponse::from(&players);
    broadcast_sse("leaderboard", res, players).await;
}
