//! Push notifications via ntfy.sh, fired through curl on a detached thread —
//! a slow network must never stall the daemon's policy loop.

pub fn push(topic: &str, title: &str, message: &str) {
    if topic.is_empty() {
        return;
    }
    let url = format!("https://ntfy.sh/{topic}");
    let title = title.to_string();
    let message = message.to_string();
    std::thread::spawn(move || {
        let _ = std::process::Command::new("curl")
            .args([
                "-s",
                "-m",
                "10",
                "-H",
                &format!("Title: {title}"),
                "-d",
                &message,
                &url,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}
