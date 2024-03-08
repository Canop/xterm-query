/// In order to query, go to raw mode, query the terminal, then leave raw mode.
///
/// This example uses crossterm because I'm used to it but you may use other
/// crates to manage raw mode.
fn query(query: &str, timeout_ms: u64) -> Result<String, xterm_query::XQError> {
    use crossterm::terminal::*;
    enable_raw_mode()?;
    let res = xterm_query::query(query, timeout_ms);
    disable_raw_mode()?;
    res
}

/// Ask the terminal whether the Kitty image protocol is supported
pub fn main() {
    let start = std::time::Instant::now();
    match query("\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c", 50) {
        Err(e) => {
            eprintln!("Error: {}", e);
            println!("(we should assume the Kitty image protocol isn't available)");
        }
        Ok(response) => {
            let kitty_support = response.starts_with("\x1b_Gi=31;OK\x1b");
            if kitty_support {
                println!("Kitty image protocol IS supported");
            } else {
                println!("Kitty image protocol is NOT supported");
            }
        }
    }
    println!("Operation took {:?}", start.elapsed());
}
