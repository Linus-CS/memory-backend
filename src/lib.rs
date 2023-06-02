pub mod queries {
    #[derive(serde::Deserialize)]
    pub struct CreateQuery {
        pub id: String,
    }

    #[derive(serde::Deserialize)]
    pub struct JoinQuery {
        pub id: String,
        pub name: String,
    }

    #[derive(serde::Deserialize)]
    pub struct PickQuery {
        pub id: String,
        pub card: usize,
    }
}

pub mod reply {
    use crate::memory::{GameState, Player};

    #[derive(serde::Serialize)]
    pub struct PickResponse {
        pub img_path: String,
        pub turn: bool,
    }

    #[derive(serde::Serialize)]
    pub struct StateResponse {
        pub game_state: GameState,
        pub ready: bool,
    }

    impl StateResponse {
        pub fn from(game_state: GameState, ready: bool) -> Self {
            Self { game_state, ready }
        }
    }

    #[derive(serde::Serialize)]
    pub struct FlipResponse {
        pub card_id: usize,
        pub img_path: String,
    }

    #[derive(serde::Serialize)]
    pub struct LeaderboardResponse {
        pub players: Vec<(String, usize, bool)>,
    }

    impl LeaderboardResponse {
        pub fn from(players: &Vec<&Player>) -> Self {
            Self {
                players: players
                    .into_iter()
                    .map(|p| (p.name.clone(), p.points, p.ready))
                    .collect(),
            }
        }
    }
}

pub mod reject {
    use std::convert::Infallible;

    use warp::{reject, Rejection, Reply};

    #[derive(Debug)]
    pub struct NoGameExists;
    impl reject::Reject for NoGameExists {}

    #[derive(Debug)]
    pub struct InvalidToken;
    impl reject::Reject for InvalidToken {}

    #[derive(Debug)]
    pub struct InvalidMasterKey;
    impl reject::Reject for InvalidMasterKey {}

    #[derive(Debug)]
    pub struct InvalidCard;
    impl reject::Reject for InvalidCard {}

    #[derive(Debug)]
    pub struct AlreadyExists;
    impl reject::Reject for AlreadyExists {}

    #[derive(Debug)]
    pub struct AlreadyRunning;
    impl reject::Reject for AlreadyRunning {}

    #[derive(Debug)]
    pub struct NotYourTurn;
    impl reject::Reject for NotYourTurn {}

    #[derive(Debug)]
    pub struct NotYetRunning;
    impl reject::Reject for NotYetRunning {}

    #[derive(Debug)]
    pub struct AlreadyFlipped;
    impl reject::Reject for AlreadyFlipped {}

    pub async fn handle_rejection(err: Rejection) -> Result<impl Reply, Infallible> {
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
}

pub mod sse_utils {
    use std::convert::Infallible;

    use warp::sse::Event;

    use crate::memory::Player;

    pub async fn broadcast_sse(
        event_name: &str,
        reply: impl serde::Serialize,
        players: Vec<&Player>,
    ) {
        for player in players {
            send_sse(event_name, &reply, player.sender.as_ref()).await;
        }
    }

    pub async fn send_sse(
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
}

pub mod memory {
    use std::{collections::HashMap, convert::Infallible, sync::Arc};

    use rand::{seq::SliceRandom, thread_rng, Rng};
    use tokio::sync::RwLock;
    use warp::{reply::Json, sse::Event, Rejection};

    use crate::{
        icons::LINKS,
        reject::{AlreadyFlipped, InvalidCard},
        reply::FlipResponse,
        sse_utils::broadcast_sse,
    };

    pub type Store = Arc<RwLock<MemoryStore>>;

    #[derive(Clone)]
    pub struct Card {
        pub img_path: String,
        pub flipped: bool,
    }

    impl Card {
        pub fn new(img_path: String) -> Self {
            Card {
                img_path,
                flipped: false,
            }
        }
    }

    pub struct Player {
        pub name: String,
        pub points: usize,
        pub turn: bool,
        pub ready: bool,
        pub sender: Option<tokio::sync::mpsc::Sender<Result<Event, Infallible>>>,
    }

    impl Player {
        pub fn new(name: String) -> Self {
            Player {
                name,
                points: 0,
                turn: false,
                ready: false,
                sender: None,
            }
        }
    }

    #[derive(serde::Serialize, Clone, Copy)]
    pub enum GameState {
        Lobby,
        Running,
        Finished,
    }

    pub struct Memory {
        pub id: String,
        pub players: HashMap<String, Player>,
        pub state: GameState,
        pub cards: Vec<Card>,
        current_turn: usize,
    }

    impl Memory {
        pub fn new(id: String) -> Self {
            let columns = 9;
            let rows = 6;
            let mut cards = Vec::with_capacity(columns * rows);
            let mut rng = thread_rng();

            let mut img = 0;
            for i in 0..columns * rows {
                cards.push(Card::new(LINKS[img].to_owned()));
                if i % 2 != 0 {
                    img += 1;
                }
            }

            cards.shuffle(&mut rng);

            Memory {
                id,
                players: HashMap::new(),
                state: GameState::Lobby,
                cards,
                current_turn: 0,
            }
        }

        pub async fn start(&mut self) {
            self.state = GameState::Running;
            let player = self.players.values_mut().nth(self.current_turn).unwrap();
            player.turn = true;
            println!("Started game.");
        }

        pub fn add_new_player(
            &mut self,
            name: String,
        ) -> Result<String, crate::reject::AlreadyExists> {
            if self.players.contains_key(&name) {
                return Err(crate::reject::AlreadyExists);
            }

            let token: String = thread_rng()
                .sample_iter(&rand::distributions::Alphanumeric)
                .take(30)
                .map(char::from)
                .collect();

            self.players
                .insert(token.clone(), Player::new(name.clone()));

            println!("{} joined and got the token: {}", name, token);
            Ok(token)
        }

        pub async fn pick_card(
            &mut self,
            card_id: usize,
            token: String,
        ) -> Result<Json, Rejection> {
            let other_card_img_path = {
                let other_card = self.cards.iter().find(|x| x.flipped);
                if let Some(card) = other_card {
                    Some(card.img_path.clone())
                } else {
                    None
                }
            };

            if let Some(card) = self.cards.get_mut(card_id) {
                if card.flipped {
                    return Err(warp::reject::custom(AlreadyFlipped));
                }
                card.flipped = true;
                let player = self.players.get_mut(&token).unwrap();
                println!("{} picked {}", player.name, card_id);

                let next = Self::check_for_pair(player, card.img_path.clone(), other_card_img_path);
                if next {
                    self.current_turn = self.current_turn + 1 % self.players.len();
                    let player = self.players.values_mut().nth(self.current_turn).unwrap();
                    player.turn = true;
                    println!("Next players turn.");
                }

                let players = self.players.values().collect();
                Self::send_flip_response(players, card.img_path.clone(), card_id).await;
                Ok(warp::reply::json(&"Success"))
            } else {
                Err(warp::reject::custom(InvalidCard))
            }
        }

        fn check_for_pair(player: &mut Player, card: String, other_card: Option<String>) -> bool {
            if let Some(other_card) = other_card {
                if card == other_card {
                    player.points += 1;
                    return false;
                } else {
                    player.turn = false;
                    return true;
                }
            }
            false
        }

        async fn send_flip_response(players: Vec<&Player>, img_path: String, card_id: usize) {
            let res = FlipResponse { img_path, card_id };
            broadcast_sse("flipCard", res, players).await
        }
    }

    #[derive(Default)]
    pub struct MemoryStore {
        pub game: Option<Memory>,
        pub master_key: String,
    }
}

pub mod icons {
    pub const LINKS: [&str; 27] = [
        "https://cdn-icons-png.flaticon.com/512/2977/2977402.png",
        "https://cdn-icons-png.flaticon.com/512/1998/1998659.png",
        "https://cdn-icons-png.flaticon.com/512/1864/1864475.png",
        "https://cdn-icons-png.flaticon.com/512/3069/3069172.png",
        "https://cdn-icons-png.flaticon.com/512/2153/2153090.png",
        "https://cdn-icons-png.flaticon.com/512/1864/1864514.png",
        "https://cdn-icons-png.flaticon.com/512/809/809052.png",
        "https://cdn-icons-png.flaticon.com/512/1998/1998610.png",
        "https://cdn-icons-png.flaticon.com/512/1864/1864470.png",
        "https://cdn-icons-png.flaticon.com/512/1998/1998713.png",
        "https://cdn-icons-png.flaticon.com/512/1998/1998627.png",
        "https://cdn-icons-png.flaticon.com/512/3196/3196017.png",
        "https://cdn-icons-png.flaticon.com/512/1067/1067840.png",
        "https://cdn-icons-png.flaticon.com/512/1864/1864472.png",
        "https://cdn-icons-png.flaticon.com/512/2977/2977327.png",
        "https://cdn-icons-png.flaticon.com/512/1010/1010028.png",
        "https://cdn-icons-png.flaticon.com/512/1998/1998804.png",
        "https://cdn-icons-png.flaticon.com/512/1864/1864521.png",
        "https://cdn-icons-png.flaticon.com/512/826/826912.png",
        "https://cdn-icons-png.flaticon.com/512/3359/3359579.png",
        "https://cdn-icons-png.flaticon.com/512/1998/1998679.png",
        "https://cdn-icons-png.flaticon.com/512/1864/1864473.png",
        "https://cdn-icons-png.flaticon.com/512/3975/3975047.png",
        "https://cdn-icons-png.flaticon.com/512/628/628341.png",
        "https://cdn-icons-png.flaticon.com/512/375/375105.png",
        "https://cdn-icons-png.flaticon.com/512/523/523495.png",
        "https://cdn-icons-png.flaticon.com/512/1531/1531395.png",
    ];
}
