use std::convert::Infallible;

use memory_backend::reply::{PickResponse, StateResponse, TurnResponse};
use rand::{thread_rng, Rng};
use tokio_stream::wrappers::ReceiverStream;
use warp::{reply::Json, sse::Event, Rejection, Reply};

use memory_backend::memory::{GameState, Memory, Player, Store};
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
    if let Some(value) = query {
        if lock.game.as_ref().unwrap().players.get(&value).is_none() {
            println!("Removed token: {}", value);
            let reply = warp::reply::with_status(reply, warp::http::StatusCode::GONE);
            return Ok(warp::reply::with_header(
                reply,
                "Set-Cookie",
                "memory_token=0; Max-Age=0, SameSite=None; Secure; HttpOnly",
            ));
        }
    }

    let reply = warp::reply::with_status(reply, warp::http::StatusCode::OK);

    Ok(warp::reply::with_header(reply, "", ""))
}

pub async fn check_key(key: String, store: Store) -> Result<impl Reply, Rejection> {
    let lock = store.read().await;
    if lock.master_key == key {
        Ok(warp::reply::with_header(
            warp::reply(),
            "Set-Cookie",
            format!(
                "master_key={}; Path=/; max-age=31536000; SameSite=None; Secure; HttpOnly",
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
    let mut lock = store.write().await;
    let game = lock.game.as_mut().unwrap();
    match game.state {
        GameState::Lobby => (),
        _ => return Err(warp::reject::custom(AlreadyRunning)),
    }

    let token: String = thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(30)
        .map(char::from)
        .collect();

    game.players
        .insert(token.clone(), Player::new(query.name.clone()));

    println!("{} joined and got the token: {}", query.name, token);

    Ok(warp::reply::with_header(
        warp::reply(),
        "set-cookie",
        format!(
            "memory_token={}; Max-Age=1209600; SameSite=None; Secure; HttpOnly",
            token
        ),
    ))
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
        Ok(warp::reply::json(&StateResponse {
            game_state: game.state,
            ready: player.ready,
        }))
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
    let player = game.players.get_mut(&token).unwrap();

    if let Some(card1) = game.cards.get_mut(query.card) {
        if card1.flipped {
            return Err(warp::reject::custom(AlreadyFlipped));
        }
        card1.flipped = true;

        if let Some(card2) = other_card {
            if card1.img_path == card2.img_path {
                player.points += 1;
            } else {
                player.turn = false;
            }
        }
        println!("{} picked {}", player.name, query.card);
        Ok(warp::reply::json(&PickResponse {
            turn: player.turn,
            img_path: card1.img_path.clone(),
        }))
    } else {
        Err(warp::reject::custom(InvalidCard))
    }
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
    println!("{} is ready", player.name);
    for (_, player) in game.players.iter() {
        if !player.ready {
            return Ok(warp::reply::json(&"Success"));
        }
    }
    game.state = GameState::Running;
    let player = game.players.values_mut().nth(0).unwrap();
    player.turn = true;
    player
        .sender
        .as_ref()
        .unwrap()
        .send(Ok(Event::default()
            .event("turn")
            .json_data(TurnResponse { turn: true })
            .unwrap_or(Event::default().comment("hello"))))
        .await
        .unwrap();

    Ok(warp::reply::json(&"Started"))
}