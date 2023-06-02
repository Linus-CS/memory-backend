use std::convert::Infallible;

use memory_backend::reply::{FlipResponse, LeaderboardResponse, StateResponse, TurnResponse};
use rand::{thread_rng, Rng};
use tokio::sync::RwLockWriteGuard;
use tokio_stream::wrappers::ReceiverStream;
use warp::reply::{WithHeader, WithStatus};
use warp::{reply::Json, sse::Event, Rejection, Reply};

use memory_backend::memory::{Card, GameState, Memory, MemoryStore, Player, Store};
use memory_backend::queries::{CreateQuery, JoinQuery, PickQuery};
use memory_backend::reject::{
    AlreadyExists, AlreadyFlipped, AlreadyRunning, InvalidCard, InvalidMasterKey, InvalidToken,
    NoGameExists, NotYetRunning, NotYourTurn,
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

pub async fn join(query: JoinQuery, store: Store) -> Result<impl Reply, Rejection> {
    let mut lock = store.write().await;
    let game = lock.game.as_mut().unwrap();

    match game.state {
        GameState::Lobby => (),
        _ => return Err(warp::reject::custom(AlreadyRunning)),
    }

    let token = create_new_player(game, query.name);
    update_leaderboard(game.players.values().collect()).await;

    set_cookie_reponse("memory_token", token)
}

pub async fn game_message(token: String, store: Store) -> Result<impl Reply, Rejection> {
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(2);

    let mut lock = store.write().await;
    let game = lock.game.as_mut().unwrap();

    game.players.get_mut(&token).unwrap().sender = Some(sender);

    let receiver_stream = ReceiverStream::new(receiver);
    let stream = warp::sse::keep_alive().stream(receiver_stream);

    Ok(warp::sse::reply(stream))
}

pub async fn state(token: String, store: Store) -> Result<Json, Rejection> {
    let lock = store.read().await;
    let game = lock.game.as_ref().unwrap();
    if let Some(player) = game.players.get(&token) {
        Ok(warp::reply::json(&StateResponse::from(
            game.state,
            player.ready,
        )))
    } else {
        Err(warp::reject::custom(InvalidToken))
    }
}

pub async fn pick_card(token: String, query: PickQuery, store: Store) -> Result<Json, Rejection> {
    let lock = store.read().await;
    let game = lock.game.as_ref().unwrap();

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

    let other_card = game.cards.iter().find(|x| x.flipped);

    let mut lock = store.write().await;
    let game = lock.game.as_mut().unwrap();

    if let Some(card) = game.cards.get_mut(query.card) {
        if card.flipped {
            return Err(warp::reject::custom(AlreadyFlipped));
        }
        card.flipped = true;
        let player = game.players.get_mut(&token).unwrap();
        println!("{} picked {}", player.name, query.card);
        check_for_pair(player, card, other_card);

        let reply = warp::reply::json(&TurnResponse { turn: player.turn });
        let players = game.players.values().collect();
        send_flip_response(players, card.img_path.clone(), query.card).await;
        Ok(reply)
    } else {
        Err(warp::reject::custom(InvalidCard))
    }
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
            return Ok(warp::reply::json(&"Success"));
        }
    }

    start_game(game).await;
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

fn create_new_player(game: &mut Memory, name: String) -> String {
    let token: String = thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(30)
        .map(char::from)
        .collect();

    game.players
        .insert(token.clone(), Player::new(name.clone()));

    println!("{} joined and got the token: {}", name, token);
    token
}

async fn update_leaderboard(players: Vec<&Player>) {
    let res = LeaderboardResponse::from(&players);
    broadcast_sse("leaderboard", res, players).await;
}

fn check_for_pair(player: &mut Player, card1: &Card, other_card: Option<&Card>) {
    if let Some(card2) = other_card {
        if card1.img_path == card2.img_path {
            player.points += 1;
        } else {
            player.turn = false;
        }
    }
}

async fn send_flip_response(players: Vec<&Player>, img_path: String, card_id: usize) {
    let res = FlipResponse { img_path, card_id };
    broadcast_sse("flipCard", res, players).await
}

async fn broadcast_sse(event_name: &str, reply: impl serde::Serialize, players: Vec<&Player>) {
    for player in players {
        send_sse(event_name, &reply, player.sender.as_ref()).await;
    }
}

async fn send_sse(
    event_name: &str,
    reply: &impl serde::Serialize,
    channel: Option<&tokio::sync::mpsc::Sender<Result<Event, Infallible>>>,
) {
    if let Some(sender) = channel {
        sender
            .send(Ok(Event::default()
                .event(event_name)
                .json_data(reply)
                .unwrap_or(Event::default().comment("hello"))))
            .await
            .unwrap();
    }
}

async fn start_game(game: &mut Memory) {
    game.state = GameState::Running;
    let player = game.players.values_mut().nth(0).unwrap();
    player.turn = true;
    send_sse("turn", &TurnResponse { turn: true }, player.sender.as_ref()).await;
}
