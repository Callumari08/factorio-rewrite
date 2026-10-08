// keys <type> <name> [path.to.key]: prints the keys and short values of a prototype node.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let data = factorio_data::load_game_data(&factorio_data::Config::load()?)?;
    let mut v = data.prototype(&a[0], &a[1]);
    if let Some(path) = a.get(2) {
        for k in path.split('.') {
            v = match k.parse::<usize>() {
                Ok(i) => v.at(i),
                Err(_) => v.get(k),
            };
        }
    }
    let n: usize = a.get(3).and_then(|x| x.parse().ok()).unwrap_or(220);
    if let Some(t) = v.as_table() {
        println!("{} keys", t.len());
        for (k, x) in t {
            let s = format!("{x:?}");
            println!("{k}: {}", &s[..s.len().min(n)]);
        }
    } else {
        let arr = v.as_array();
        println!("array len {}", arr.len());
        for (i, f) in arr.iter().enumerate().take(4) {
            let s = format!("{f:?}");
            println!("[{i}]: {}", &s[..s.len().min(n)]);
        }
    }
    Ok(())
}
