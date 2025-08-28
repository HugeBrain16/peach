use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use regex::Regex;
use chrono::{DateTime, Local};

const HOST: &'static str = "0.0.0.0";
const PORT: u32 = 5000;
const BUFFER_SIZE: usize = 256;
const BUFFER_COUNT: usize = 100;

const CLEAR: &str = "\x1b[2J\x1b[H";
const RESET: &str = "\x1b[0m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";

struct Channel {
    log: Arc<Mutex<String>>,
    tx: broadcast::Sender<String>
}

enum Client {
    Netcat, 
}

struct User {
    name: String,
    channel: String,
    client: Client
}

type Channels = Arc<Mutex<HashMap<String, Channel>>>;

fn get_timestamp() -> String {
    let now: DateTime<Local> = Local::now();
    
    return format!("{}", now.format("%d-%m-%Y_%H:%M:%S"));
}

async fn join_channel(channels: Channels, name: &str) -> (broadcast::Receiver<String>, Arc<Mutex<String>>) {
    let mut rooms = channels.lock().await;

    let channel = rooms.entry(name.to_string()).or_insert_with(|| { 
        let (tx, _) = broadcast::channel(BUFFER_COUNT);

        return Channel {
            log: Arc::new(Mutex::new(String::new())),
            tx
        };
    });

    return (channel.tx.subscribe(), channel.log.clone());
}

async fn read_message(stream: &mut TcpStream, message: &mut String) -> u32 {
    let mut buffer = [0; BUFFER_SIZE];
    let mut total_bytes: u32 = 0;

    loop {
        let bytes = stream.read(&mut buffer).await.unwrap();

        message.push_str(&String::from_utf8_lossy(&mut buffer[..bytes]));
        total_bytes += bytes as u32;

        if bytes == 0 || message.ends_with("\n") || message.ends_with("\n\r") {
            break;
        }
    }

    return total_bytes;
}

async fn broadcast(channels: &Channels, name: &str, message: String) {
    let mut chans = channels.lock().await;

    if let Some(chan) = chans.get_mut(name) {
        let _ = chan.tx.send(message);
    }
}

async fn handle_stream(mut stream: TcpStream, mut user: User, channels: Channels) {
    let (mut rx, mut log) = join_channel(channels.clone(), &user.channel).await;
    let mut message = String::new();

    let line = format!("{}[{}][{}] joined!{}\n", GREEN, get_timestamp(), user.name, RESET);
    {
        let mut logs = log.lock().await;
        logs.push_str(&line);
        broadcast(&channels, &user.channel, logs.clone()).await;
    }
    
    loop {
        tokio::select! {
            Ok(msg) = rx.recv() => {
                let _ = stream.write(format!("{}\n{}\n#[{}] Type your message: ", CLEAR, msg, user.channel).as_bytes()).await;
            }
            
            bytes = read_message(&mut stream, &mut message) => {
                if bytes == 0 {
                    let line = format!("{}[{}][{}] Left{}\n", RED, get_timestamp(), user.name, RESET);
                    { 
                        let mut logs = log.lock().await;
                        logs.push_str(&line);

                        match user.client {
                            Client::Netcat => { broadcast(&channels, &user.channel, logs.clone()).await; }
                        }
                    }
                    break;
                }

                if message.starts_with("/") {
                    let cmd: Vec<&str> = message.trim_end().split_whitespace().collect();

                    if cmd[0] == "/join" {
                        let line = format!("{}[{}][{}] Left{}\n", RED, get_timestamp(), user.name, RESET);
                        { 
                            let mut logs = log.lock().await;
                            logs.push_str(&line);

                            match user.client {
                                Client::Netcat => { broadcast(&channels, &user.channel, logs.clone()).await; }
                            }
                        }

                        (rx, log) = join_channel(channels.clone(), cmd[1]).await;
                        user.channel = cmd[1].to_string(); 

                        let line = format!("{}[{}][{}] joined!{}\n", GREEN, get_timestamp(), user.name, RESET);
                        {
                            let mut logs = log.lock().await;
                            logs.push_str(&line);
                            broadcast(&channels, &user.channel, logs.clone()).await;
                        }
                    } else {
                        let _ = stream.write(b"[Server] Command not found!\n").await;
                    }
                } else {
                    let line = format!("[{}][{}]: {}\n", get_timestamp(), user.name, message.trim_end());
                    {
                        let mut logs = log.lock().await;
                        logs.push_str(&line);  
                    
                        match user.client {
                            Client::Netcat => { broadcast(&channels, &user.channel, logs.clone()).await; }
                        }
                    }
                }
                 
                message.clear();
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let name_regex = Regex::new("^[a-zA-Z0-9_-]+$").unwrap();
    let address = format!("{}:{}", HOST, PORT);
    let listener = TcpListener::bind(&address).await.unwrap();
    let channels = Arc::new(Mutex::new(HashMap::new()));
    println!("Peach is listening on {}...", address);

    loop {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut name = String::new();
        let mut client_msg: String = String::new();
        let client;
 
        loop {
            let _ = stream.write(b"[Server] Welcome to Peach!\nThis is an anonymous chat server, use '/join <channel>' to join a channel\nPlease enter your name: ").await;
        
            read_message(&mut stream, &mut name).await;
            name = name.trim().to_string();
        
            if name_regex.is_match(&name) {
                break;
            }
            
            let _ = stream.write(format!("{}\n[Server] Name you entered is invalid!\n", CLEAR).as_bytes()).await;
            name.clear();
        }

        loop {
            let _ = stream.write(format!("{}\n[Server] Which client are you using?\n\t1). Netcat\nSelect (default=1): ", CLEAR).as_bytes()).await;
            
            read_message(&mut stream, &mut client_msg).await;
            client_msg = client_msg.trim_end().to_string();

            if client_msg == "1" || client_msg.is_empty() {
                client = Client::Netcat;
                break;
            }
        }

        let user = User {
            name: name,
            channel: "general".to_string(),
            client: client
        };

        tokio::spawn(handle_stream(stream, user, channels.clone()));
    }
}
