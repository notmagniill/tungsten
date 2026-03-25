use std::io::Write;

#[macro_export]
macro_rules! log {
    ($level:ident, $($arg:tt)*) => {{
        use colored::Colorize;
        let msg = format!($($arg)*);
        match stringify!($level) {
            "info" => println!("{} {}", "∙".white().bold(), msg.white()),
            "success" => println!("{} {}", "✓".green().bold(), msg.green()),
            "warn" => println!("{} {}", "⚠".yellow().bold(), msg.yellow()),
            "error" => println!("{} {}", "✗".red().bold(), msg.red()),
            "section" => println!(
                "\n{} {}{}",
                "──".cyan().bold(),
                msg.cyan().bold(),
                " ──".cyan().bold()
            ),
            _ => println!("{}", msg),
        }
    }};
}

pub fn info(msg: &str) {
    log!(info, "{}", msg)
}
pub fn success(msg: &str) {
    log!(success, "{}", msg)
}
pub fn warn(msg: &str) {
    log!(warn, "{}", msg)
}
pub fn error(msg: &str) {
    log!(error, "{}", msg)
}
pub fn section(msg: &str) {
    log!(section, "{}", msg)
}

pub fn progress(current: usize, total: usize, label: &str) {
    use colored::Colorize;

    let bar_width = 30;
    let filled = ((current as f32 / total as f32) * bar_width as f32).floor() as usize;
    let empty = bar_width - filled;
    let percent = ((current as f32 / total as f32) * 100.0).floor() as usize;

    let bar = format!(
        "{}{}",
        "█".repeat(filled).green().bold().to_string(),
        "░".repeat(empty).dimmed().to_string(),
    );

    let line = format!(
        "[{}] {} {}/{} {:<40}",
        bar,
        format!("{}%", percent).cyan().bold(),
        format!("{}", current).bold(),
        format!("{}", total).bold(),
        label.dimmed(),
    );

    if current >= total {
        println!("\r{}", line);
    } else {
        print!("\r{}", line);
        std::io::stdout().flush().unwrap();
    }
}