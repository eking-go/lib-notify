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
        if self.qs != 0 {
            let mta = MessageToAgent {
                message: "".to_string(),
                action: StopAgent::Immediately,
            };
            let _ = self.sender.send(mta);
        }
    }

    pub fn stop_agent_when_queue_is_empty(&self) {
        if self.qs != 0 {
            let mta = MessageToAgent {
                message: "".to_string(),
                action: StopAgent::WhenQueueIsEmpty,
            };
            let _ = self.sender.send(mta);
        }
    }

    pub fn send_notify(&self, msg: &String) {
        if msg.is_empty() {
            return;
        };
        let result = generic_sender(&self.agent_type, &self.app_name, msg);
        match result {
            Ok(()) => (),
            Err(_) => {
                if self.qs != 0 {
                    println!("NotifyAgent: Move message to the queue");
                    let mta = MessageToAgent {
                        message: msg.to_string(),
                        action: StopAgent::Empty,
                    };
                    match self.sender.send(mta) {
                        Ok(()) => (),
                        Err(e) => {
                            println!("NotifyAgent: Can't move message {} to queue! {}", msg, e)
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
    if qs == 0 {
        return;
    };
    let mut dmq: Queue<String> = queue![];
    let sleep_delay = time::Duration::from_secs(1);
    let mut last_attempt = Local::now();
    'main_loop: loop {
        // first of all we are trying to empty queue
        let lad = Local::now() - last_attempt;
        if lad.num_seconds() > rds.try_into().unwrap() {
            last_attempt = Local::now();
            loop {
                match dmq.peek() {
                    Ok(q) => match generic_sender(agent_type, app_name, &q) {
                        Ok(()) => {
                            let _ = dmq.remove();
                            let cqsize = dmq.size();
                            println!(
                                "NotifyAgent: Delayed message has been sent, queue size : {cqsize}"
                            );
                        }
                        Err(_) => {
                            let cqsize = dmq.size();
                            println!("NotifyAgent: Unable to resend message from queue, queue size is {cqsize}");
                            break;
                        }
                    },
                    Err(_) => break,
                };
            }
        };
        // trying to get new messages
        loop {
            let (msg, action) = match receiver.try_recv() {
                Ok(m) => (m.message, m.action),
                Err(e) => match e {
                    std::sync::mpsc::TryRecvError::Empty => {
                        // If we don't receive new messages - just sleep and try one more time later
                        break;
                    }
                    std::sync::mpsc::TryRecvError::Disconnected => {
                        println!("NotifyAgent: Receiver disconnected: {}\nExiting...", e);
                        break 'main_loop;
                    }
                },
            };
            // if we receive new messages - checking if we should exit
            match action {
                StopAgent::WhenQueueIsEmpty => {
                    if dmq.size() == 0 {
                        break 'main_loop;
                    };
                }
                StopAgent::Empty => (),
                StopAgent::Immediately => break 'main_loop,
            };
            // if queue size is equal max queue size, so, we don't have place to add one more message
            // then we just remove oldest message
            if dmq.size() == qs {
                match dmq.remove() {
                    Ok(dm) => {
                        println!("NotifyAgent: Queue overflow - remove the oldest message: {dm}")
                    }
                    Err(_) => println!("NotifyAgent: Strange, queue should not be empty..."),
                };
            };
            let dnow: DateTime<Local> = Local::now();
            let dmr = format!(
                "Message has been delayed since:\n{}\n{}",
                dnow.format("%Y-%m-%d %H:%M %:z"),
                &msg
            );
            match dmq.add(dmr) {
                Ok(_) => println!("NotifyAgent: Added "),
                Err(e) => println!("NotifyAgent: Unable to add to internal queue: {}", e),
            };
        }
        thread::sleep(sleep_delay);
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
