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

    #[derive(serde::Serialize)]
    pub struct PickResponse {
        pub img_path: String,
        pub turn: bool,
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
    pub struct AlreadyExists;
    impl reject::Reject for AlreadyExists {}

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

pub mod memory {
    use std::{collections::HashMap, convert::Infallible, sync::Arc};

    use rand::{seq::SliceRandom, thread_rng};
    use tokio::sync::{
        mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
        RwLock,
    };
    use warp::sse::Event;

    use crate::icons::LINKS;

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
        pub channel: (
            UnboundedSender<Result<Event, Infallible>>,
            UnboundedReceiver<Result<Event, Infallible>>,
        ),
    }

    impl Player {
        pub fn new(name: String) -> Self {
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
            }
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
