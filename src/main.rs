use anyhow::{Error, Result};
use clap::{App, Arg};
use crossterm::{
    self,
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, size, EnterAlternateScreen, LeaveAlternateScreen,
        SetSize,
    },
};
use dirs;
use std::io::{stdout, Write};
use terki::{Location, Terki};
use tokio;

async fn run(terki: &mut Terki, wiki: Option<&str>) -> Result<(), Error> {
    if let Some(wiki) = wiki {
        terki
            .display(wiki, "welcome-visitors", Location::End)
            .await?;
    } else {
        terki.display_active_pane()?;
    }
    terki.handle_input().await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let matches = App::new("terki")
        .arg(Arg::with_name("url").long("url").takes_value(true))
        .arg(Arg::with_name("local").long("local").takes_value(true))
        .get_matches();
    let size = size()?;
    let mut terki = Terki::new((size.0 as usize, size.1 as usize));
    terki.load().await?;
    let wiki = if let Some(path) = matches.value_of("local") {
        let mut wikidir = dirs::home_dir()
            .expect("unable to get home dir")
            .join(".wiki");
        if !wikidir.exists() {
            println!("~/.wiki does not exist!");
            std::process::exit(1);
        }
        if path != "localhost" {
            wikidir = wikidir.join(path);
            if !wikidir.exists() {
                println!("{} does not exist!", wikidir.display());
                std::process::exit(1);
            }
        }
        terki.add_local(wikidir).expect("Unable to add local wiki!");
        Some(path.to_owned())
    } else if let Some(url) = matches.value_of("url") {
        Some(terki.add_remote(url)?)
    } else if terki.wikis.len() == 0 {
        println!("Must pass in at least one of: --url or --local");
        std::process::exit(1);
    } else {
        None
    };

    enable_raw_mode()?;
    let mut stdout = stdout();
    println!("{}, {}", size.0, size.1);
    execute!(
        stdout,
        EnterAlternateScreen,
        SetSize(size.0, size.1 + 1000),
        EnableMouseCapture
    )?;
    let result = run(&mut terki, wiki.as_deref()).await;
    execute!(stdout, LeaveAlternateScreen, DisableMouseCapture)?;
    disable_raw_mode()?;
    terki.save()?;
    result
}
