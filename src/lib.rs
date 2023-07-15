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

    pub type Players = Vec<(String, usize, bool, bool)>;

    #[derive(serde::Serialize)]
    pub struct PickResponse {
        pub img_path: String,
        pub turn: bool,
    }

    #[derive(serde::Serialize)]
    pub struct HideResponse {
        pub card_id: usize,
    }

    #[derive(serde::Serialize)]
    pub struct GameOverResponse {
        pub game_state: GameState,
    }

    #[derive(serde::Serialize)]
    pub struct InitResponse {
        pub game_state: GameState,
        pub ready: bool,
        pub flipped: Vec<(usize, String)>,
        pub hidden: Vec<usize>,
        pub players: Players,
    }

    impl InitResponse {
        pub fn from(
            game_state: GameState,
            ready: bool,
            flipped: Vec<(usize, String)>,
            hidden: Vec<usize>,
            players: Players,
        ) -> Self {
            Self {
                game_state,
                ready,
                flipped,
                hidden,
                players,
            }
        }
    }

    #[derive(serde::Serialize)]
    pub struct FlipResponse {
        pub card_id: usize,
        pub img_path: String,
    }

    #[derive(serde::Serialize)]
    pub struct LeaderboardResponse {
        pub players: Players,
    }

    impl LeaderboardResponse {
        pub fn from(players: &Vec<&Player>) -> Self {
            Self {
                players: players
                    .into_iter()
                    .map(|p| (p.name.clone(), p.points, p.ready, p.turn))
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
        reply::{FlipResponse, GameOverResponse, HideResponse, InitResponse},
        sse_utils::broadcast_sse,
    };

    pub type Store = Arc<RwLock<MemoryStore>>;

    #[derive(Clone)]
    pub struct Card {
        pub img_path: String,
        pub flipped: bool,
        pub gone: bool,
    }

    impl Card {
        pub fn new(img_path: String) -> Self {
            Card {
                img_path,
                flipped: false,
                gone: false,
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

            let (mut next, mut pair) = (false, false);

            let reply = if let Some(card) = self.cards.get_mut(card_id) {
                if card.flipped || card.gone {
                    return Err(warp::reject::custom(AlreadyFlipped));
                }
                card.flipped = true;
                let player = self.players.get_mut(&token).unwrap();
                println!("{} picked {}", player.name, card_id);

                (next, pair) =
                    Self::check_for_pair(player, card.img_path.clone(), other_card_img_path);

                let players = self.players.values().collect();
                Self::send_flip_response(players, card.img_path.clone(), card_id).await;
                Ok(warp::reply::json(&"Success"))
            } else {
                Err(warp::reject::custom(InvalidCard))
            };

            if pair {
                for (i, card) in self.cards.iter_mut().enumerate() {
                    if pair && card.flipped {
                        card.gone = true;
                        card.flipped = false;
                        Self::send_hide_response(self.players.values().collect(), i).await;
                    }
                }
                if self.cards.iter().all(|x| x.gone) {
                    self.state = GameState::Finished;
                    broadcast_sse(
                        "gameOver",
                        GameOverResponse {
                            game_state: self.state,
                        },
                        self.players.values().collect(),
                    )
                    .await;
                }
            }
            if next {
                self.next_turn();
            }

            reply
        }

        pub fn get_state(&self, ready: bool) -> InitResponse {
            let flipped = self
                .cards
                .iter()
                .enumerate()
                .filter(|(_, x)| x.flipped)
                .map(|(i, c)| (i, c.img_path.clone()))
                .collect::<Vec<_>>();
            let hidden = self
                .cards
                .iter()
                .enumerate()
                .filter(|(_, x)| x.gone)
                .map(|(i, _)| i)
                .collect::<Vec<_>>();

            let players = self
                .players
                .values()
                .into_iter()
                .map(|p| (p.name.clone(), p.points, p.ready, p.turn))
                .collect();

            InitResponse::from(self.state, ready, flipped, hidden, players)
        }

        fn next_turn(&mut self) {
            self.current_turn = (self.current_turn + 1) % self.players.len();
            let player = self.players.values_mut().nth(self.current_turn).unwrap();
            player.turn = true;
            for card in self.cards.iter_mut() {
                card.flipped = false;
            }
            println!("Next players turn.");
        }

        fn check_for_pair(
            player: &mut Player,
            card: String,
            other_card: Option<String>,
        ) -> (bool, bool) {
            if let Some(other_card) = other_card {
                if card == other_card {
                    player.points += 1;
                    return (false, true);
                } else {
                    player.turn = false;
                    return (true, false);
                }
            }
            (false, false)
        }

        async fn send_flip_response(players: Vec<&Player>, img_path: String, card_id: usize) {
            let res = FlipResponse { img_path, card_id };
            broadcast_sse("flipCard", res, players).await
        }

        async fn send_hide_response(players: Vec<&Player>, card_id: usize) {
            let res = HideResponse { card_id };
            broadcast_sse("hideCard", res, players).await
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
        "https://www.zooplus.de/magazin/wp-content/uploads/2021/04/AdobeStock_175183320-1536x1023.jpeg",
        "https://www.thesportsman.com/media/images/admin/football/original/Ronaldo_WORLDIE.jpg",
        "https://ichef.bbci.co.uk/news/976/cpsprodpb/4B2E/production/_112764291_minecraft.jpg",
        "https://cdn-icons-png.flaticon.com/512/3069/3069172.png",
        "https://bgr.com/wp-content/uploads/2015/08/darth-vader.jpg",
        "https://external-content.duckduckgo.com/iu/?u=http%3A%2F%2Fbilder.4ever.eu%2Fdata%2Fdownload%2Ftiere%2Fhaschen%2Fhasen-154275.jpg&f=1&nofb=1&ipt=a5b5801561db52435e33491017f6aaa394b5460d4f5c3dc4f84c0af9ddb97695&ipo=images",
        "https://cdn-icons-png.flaticon.com/512/809/809052.png",
        "https://cdn-icons-png.flaticon.com/512/1998/1998610.png",
        "https://cdn-icons-png.flaticon.com/512/1864/1864470.png",
        "https://cdn-icons-png.flaticon.com/512/1998/1998713.png",
        "https://cdn-icons-png.flaticon.com/512/1998/1998627.png",
        "https://cdn-icons-png.flaticon.com/512/3196/3196017.png",
        "https://cdn-icons-png.flaticon.com/512/1067/1067840.png",
        "https://img1.cgtrader.com/items/4013852/70e58e37f4/large/stumble-guys-inferno-dragon-3d-model-stl.jpg",
        "https://cdn-icons-png.flaticon.com/512/2977/2977327.png",
        "https://cdn-icons-png.flaticon.com/512/1010/1010028.png",
        "https://cdn-icons-png.flaticon.com/512/1998/1998804.png",
        "https://cdn-icons-png.flaticon.com/512/1864/1864521.png",
        "https://cdn-icons-png.flaticon.com/512/826/826912.png",
        "http://www.nintendoworldreport.com/media/29905/4/32.jpg",
        "https://cdn-icons-png.flaticon.com/512/1998/1998679.png",
        "https://cdn-icons-png.flaticon.com/512/1864/1864473.png",
        "https://cdn-icons-png.flaticon.com/512/3975/3975047.png",
        "https://cdn-icons-png.flaticon.com/512/628/628341.png",
        "https://cdn-icons-png.flaticon.com/512/375/375105.png",
        "https://manga-mafia.de/media/catalog/product/cache/2/image/9df78eab33525d08d6e5fb8d27136e95/a/b/abydco456.jpg",
        "https://cdn.pixabay.com/photo/2022/07/09/22/16/michael-jordan-7311821_960_720.png",
    ];
}
