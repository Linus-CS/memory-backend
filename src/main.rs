use std::{collections::HashMap, convert::Infallible, env, sync::Arc};

use rand::{seq::SliceRandom, thread_rng, Rng};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    RwLock,
};
use tokio_stream::wrappers::ReceiverStream;
use warp::{reject, reply::Json, sse::Event, Filter, Rejection, Reply};

#[derive(serde::Deserialize)]
struct CreateQuery {
    id: String,
}

#[derive(serde::Deserialize)]
struct JoinQuery {
    id: String,
    name: String,
}

#[derive(serde::Deserialize)]
struct PickQuery {
    id: String,
    card: usize,
}

#[derive(Debug)]
struct NoGameExists;
impl reject::Reject for NoGameExists {}

#[derive(Debug)]
struct InvalidToken;
impl reject::Reject for InvalidToken {}

#[derive(Debug)]
struct InvalidMasterKey;
impl reject::Reject for InvalidMasterKey {}

#[derive(Debug)]
struct AlreadyExists;
impl reject::Reject for AlreadyExists {}

#[derive(Clone)]
struct Card {
    pub pair_with: usize,
    pub img_path: String,
    pub flipped: bool,
}

struct Player {
    pub name: String,
    pub points: usize,
    pub turn: bool,
    pub ready: bool,
    pub channel: (
        UnboundedSender<Result<Event, Infallible>>,
        UnboundedReceiver<Result<Event, Infallible>>,
    ),
}

impl Player {
    fn new(name: String) -> Self {
        let channel = unbounded_channel::<Result<Event, Infallible>>();
        Player {
            name,
            points: 0,
            turn: false,
            ready: false,
            channel,
        }
    }
}

enum GameState {
    Lobby,
    Running,
    Finished,
}

struct Memory {
    pub id: String,
    pub players: HashMap<String, Player>,
    pub state: GameState,
    pub cards: Vec<Card>,
}

impl Memory {
    fn new(id: String) -> Self {
        let columns = 9;
        let rows = 6;
        let mut cards = Vec::with_capacity(columns * rows);
        let mut rng = thread_rng();

        for i in 0..(columns * rows / 2) {
            cards.push(Card {
                pair_with: i,
                // Image path needs to be figured out
                img_path: format!("{}.png", id),
                flipped: false,
            });
        }

        cards.extend(cards.clone());
        cards.shuffle(&mut rng);

        Memory {
            id,
            players: HashMap::new(),
            state: GameState::Lobby,
            cards,
        }
    }
}

#[derive(Default)]
struct MemoryStore {
    pub game: Option<Memory>,
    pub master_key: String,
}

type Store = Arc<RwLock<MemoryStore>>;

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
        master_key: key,
    }));
    let store = warp::any().map(move || store.clone());

    let ping_route = warp::get()
        .and(warp::cookie::optional("memory_token"))
        .and(warp::path("ping"))
        .and(warp::path::end())
        .and(store.clone())
        .and_then(ping);

    let create_route = warp::post()
        .and(warp::cookie("master_key"))
        .and(warp::path("create"))
        .and(warp::query::<CreateQuery>())
        .and(warp::path::end())
        .and(store.clone())
        .and_then(create);

    let join_route = warp::post()
        .and(warp::path("join"))
        .and(warp::query::<JoinQuery>())
        .and(warp::path::end())
        .and(store.clone())
        .and_then(join);

    let game_route = warp::get()
        .and(warp::path("game"))
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
        .or(create_route)
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

async fn handle_rejection(err: Rejection) -> Result<impl Reply, Infallible> {
    if err.find::<InvalidToken>().is_some() {
        eprintln!("Invalid token");
        return Ok(warp::reply::with_status(
            "Invalid token",
            warp::http::StatusCode::UNAUTHORIZED,
        ));
    }

    if err.find::<InvalidMasterKey>().is_some() {
        eprintln!("Invalid master key");
        return Ok(warp::reply::with_status(
            "Invalid master key",
            warp::http::StatusCode::UNAUTHORIZED,
        ));
    }

    if err.find::<AlreadyExists>().is_some() {
        eprintln!("Game already exists");
        return Ok(warp::reply::with_status(
            "Game already exists",
            warp::http::StatusCode::CONFLICT,
        ));
    }

    if err.find::<NoGameExists>().is_some() {
        eprintln!("No game exists");
        return Ok(warp::reply::with_status(
            "No game exists",
            warp::http::StatusCode::NOT_FOUND,
        ));
    }

    eprintln!("Unhandled rejection: {:?}", err);
    Ok(warp::reply::with_status(
        "Internal server error",
        warp::http::StatusCode::INTERNAL_SERVER_ERROR,
    ))
}

async fn ping(token: Option<String>, store: Store) -> Result<impl Reply, Rejection> {
    let lock = store.read().await;
    if lock.game.is_none() {
        return Err(warp::reject::custom(NoGameExists));
    }

    let reply = warp::reply::json(&lock.game.as_ref().unwrap().id);
    if let Some(value) = token {
        if lock.game.as_ref().unwrap().players.get(&value).is_none() {
            println!("Removed token: {}", value);
            return Ok(warp::reply::with_header(
                reply,
                "set-cookie",
                "memory_token=0; Path=/; SameSite=Strict; HostOnly=false; Max-Age=0",
            ));
        }
    }

    Ok(warp::reply::with_header(
        reply,
        "Access-Control-Allow-Origin",
        "*",
    ))
}

async fn create(master_key: String, query: CreateQuery, store: Store) -> Result<Json, Rejection> {
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

async fn join(query: JoinQuery, store: Store) -> Result<impl Reply, Rejection> {
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
            "memory_token={}; Path=/; SameSite=Strict; HostOnly=false; Max-Age=1209600",
            token
        ),
    ))
}

async fn game_message(store: Store) -> Result<impl Reply, Rejection> {
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(2);
    let receiverStream = ReceiverStream::new(receiver);
    let stream = warp::sse::keep_alive().stream(receiverStream);
    Ok(warp::sse::reply(stream))
}

async fn pick_card(token: String, query: PickQuery, store: Store) -> Result<Json, Rejection> {
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

async fn ready(token: String, store: Store) -> Result<Json, Rejection> {
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
