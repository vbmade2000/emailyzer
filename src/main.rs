use crate::database::create_or_open_db;

mod database;

fn main() -> anyhow::Result<()> {
    let _conn = create_or_open_db("emailyzer.db")?;

    Ok(())
}
