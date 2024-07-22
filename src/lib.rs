use chrono::{DateTime, Local};
use gethostname::gethostname;
use queues::*;
use rustelebot::types::ErrorResult;
use rustelebot::types::{SendMessageOption, SendMessageParseMode};
use rustelebot::{create_instance, send_message};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::{spawn, JoinHandle};
use std::{fmt, thread, time};

#[derive(Debug, Clone)]
pub struct NotifyError {
    pub message: String,
}

// Mandatory
impl fmt::Display for NotifyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.message)
    }
}

// Mandatory - std::error::Error.
impl std::error::Error for NotifyError {
    fn description(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type", content = "conf")]
pub enum AgentType {
    Telegram(TelegramConfig),
}

impl Default for AgentType {
    fn default() -> Self {
        AgentType::Telegram(TelegramConfig::default())
    }
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct TelegramConfig {
    pub token: String,
    pub chat: String,
}

enum StopAgent {
    Immediately,
    WhenQueueIsEmpty,
    Empty,
}

struct MessageToAgent {
    message: String,
    action: StopAgent,
}

#[derive(Clone)]
pub struct NotifyAgent {
    agent_type: AgentType,
    app_name: String,
    sender: Sender<MessageToAgent>,
    qs: usize,
}

impl NotifyAgent {
    pub fn new(at: &AgentType, app_name: &String, rds: u64, qs: usize) -> (Self, JoinHandle<()>) {
        let (sender, receiver) = channel();
        let atc = at.clone();
        let app_n = app_name.clone();
        let h = spawn(move || sender_queue(receiver, &atc, &app_n, rds.clone(), qs.clone()));
        let na = NotifyAgent {
            agent_type: at.clone(),
            app_name: app_name.clone(),
            sender: sender,
            qs: qs.clone(),
        };
        (na, h)
    }

    pub fn stop_agent_immediately(&self) {
        let mta = MessageToAgent {
            message: "".to_string(),
            action: StopAgent::Immediately,
        };
        let _ = self.sender.send(mta);
    }

    pub fn stop_agent_when_queue_is_empty(&self) {
        let mta = MessageToAgent {
            message: "".to_string(),
            action: StopAgent::WhenQueueIsEmpty,
        };
        let _ = self.sender.send(mta);
    }

    pub fn send_notify(&self, msg: &String) {
        let result = generic_sender(&self.agent_type, &self.app_name, msg);
        match result {
            Ok(()) => (),
            Err(_) => {
                if self.qs != 0 {
                    println!("Move message to the queue");
                    let mta = MessageToAgent {
                        message: msg.to_string(),
                        action: StopAgent::Empty,
                    };
                    match self.sender.send(mta) {
                        Ok(()) => (),
                        Err(e) => {
                            println!("Can't move message {} to queue! {}", msg, e)
                        }
                    };
                };
            }
        }
    }
}

//===========================================================================
fn sender_queue(
    receiver: Receiver<MessageToAgent>,
    agent_type: &AgentType,
    app_name: &String,
    rds: u64,
    qs: usize,
) {
    let mut dmq: Queue<String> = queue![];
    let mut exit_flag = false;
    let sleep_delay = time::Duration::from_secs(rds);
    loop {
        let dnow: DateTime<Local> = Local::now();
        // first of all we are trying to empty queue
        loop {
            match dmq.peek() {
                Ok(q) => match generic_sender(agent_type, app_name, &q) {
                    Ok(()) => {
                        let _ = dmq.remove();
                    }
                    Err(_) => break,
                },
                Err(_) => break,
            };
        }
        if dmq.size() == 0 && exit_flag {
            break;
        };
        thread::sleep(sleep_delay);
        // trying to get new messages
        let (msg, action) = match receiver.try_recv() {
            Ok(m) => (m.message, m.action),
            Err(e) => match e {
                std::sync::mpsc::TryRecvError::Empty => continue,
                std::sync::mpsc::TryRecvError::Disconnected => {
                    println!("Receiver disconnected: {}", e);
                    break;
                }
            },
        };
        match action {
            StopAgent::WhenQueueIsEmpty => {
                exit_flag = true;
            }
            StopAgent::Empty => (),
            StopAgent::Immediately => break,
        };
        if msg.is_empty() {
            continue;
        };
        if dmq.size() == qs {
            let _ = dmq.remove();
        };
        let dmr = format!(
            "Message has been delayed since:\n{}\n{}",
            dnow.format("%Y-%m-%d %H:%M %:z"),
            &msg
        );
        match dmq.add(dmr) {
            Ok(_) => (),
            Err(e) => println!("Unable to add to internal queue: {}", e),
        };
    }
}

//===========================================================================
fn generic_sender(
    agent_type: &AgentType,
    app_name: &String,
    msg: &String,
) -> Result<(), ErrorResult> {
    match agent_type {
        AgentType::Telegram(tc) => tg_send_msg(app_name, msg, &tc),
    }
}

//===========================================================================
fn tg_send_msg(app_name: &String, s: &String, tc: &TelegramConfig) -> Result<(), ErrorResult> {
    let mut ss = String::from(format!(
        "*{}* on `{}`\n",
        app_name,
        gethostname().to_string_lossy()
    ));
    let now: DateTime<Local> = Local::now();
    ss = ss + &format!("`{}`\n", now.format("%Y-%m-%d %H:%M %:z")) + &s;
    let message = ss
        .replace(".", r"\.")
        .replace("-", r"\-")
        .replace("!", r"\!")
        .replace("~", r"\~")
        .replace("+", r"\+")
        .replace("(", r"\(")
        .replace(")", r"\)")
        .replace(">", r"\>")
        .replace("=", r"\=")
        .replace("_", r"\_");
    let instance = create_instance(&tc.token, &tc.chat);
    let option = SendMessageOption {
        parse_mode: Some(SendMessageParseMode::MarkdownV2),
    };
    match send_message(&instance, &message, Some(option)) {
        Ok(_) => return Ok(()),
        Err(e) => {
            println!("Unable to send message: {} - {}", e, message);
            return Err(e);
        }
    };
}
//===========================================================================
