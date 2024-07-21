# Lib-Notify

Rust lib to send notify (and resend it later if network unavailable)

Right now only telegram is supporting.

```
[dependencies]
lib-notify = { git = "https://github.com/eking-go/lib-notify.git" }
```

```
use lib_notify::{AgentType, TelegramConfig, NotifyAgent};
use std::{thread, time};

fn main() {
    let tc = TelegramConfig {
      token: "...Q".to_string(),
      chat: "00000".to_string()
    };
    let at = AgentType::Telegram(tc);
    // NotifyAgent::new(&AgentType, &"application name", resend_delay_in_seconds : <u64>, max_number_delayed_messages_in_queue: <usize>)
    let (na, _) = NotifyAgent::new(&at, &"Application name".to_string(), 10, 10);
    let sleep_delay = time::Duration::from_secs(5);
    let mut c = 0;
    loop {
      thread::sleep(sleep_delay);
      let m = format!("Message number: {}", c);
      println!("-------- {}", &m);
      na.send_notify(&m);
      c += 1;
    }
}
```
