use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct Transcript {
    pub id: i64,
    pub created_at: i64,
    pub title: String,
    pub text: String,
    pub raw_text: String,
    pub language: String,
    pub model: String,
    pub style: String,
    pub duration_ms: i64,
    pub words: i64,
    pub favorite: bool,
    pub has_audio: bool,
}

pub struct NewTranscript<'a> {
    pub title: &'a str,
    pub text: &'a str,
    pub raw_text: &'a str,
    pub language: &'a str,
    pub model: &'a str,
    pub style: &'a str,
    pub duration_ms: i64,
    pub audio_path: Option<&'a str>,
}

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS transcripts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            created_at INTEGER NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            text TEXT NOT NULL,
            raw_text TEXT NOT NULL DEFAULT '',
            language TEXT NOT NULL DEFAULT '',
            model TEXT NOT NULL DEFAULT '',
            style TEXT NOT NULL DEFAULT '',
            duration_ms INTEGER NOT NULL DEFAULT 0,
            words INTEGER NOT NULL DEFAULT 0,
            favorite INTEGER NOT NULL DEFAULT 0,
            audio_path TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_transcripts_created ON transcripts(created_at);",
    )?;
    Ok(conn)
}

pub fn kv_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM kv WHERE key = ?1", params![key], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

pub fn kv_set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO kv(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn row_to_transcript(r: &rusqlite::Row) -> rusqlite::Result<Transcript> {
    let audio: Option<String> = r.get(11)?;
    Ok(Transcript {
        id: r.get(0)?,
        created_at: r.get(1)?,
        title: r.get(2)?,
        text: r.get(3)?,
        raw_text: r.get(4)?,
        language: r.get(5)?,
        model: r.get(6)?,
        style: r.get(7)?,
        duration_ms: r.get(8)?,
        words: r.get(9)?,
        favorite: r.get::<_, i64>(10)? != 0,
        has_audio: audio.is_some(),
    })
}

const COLS: &str =
    "id, created_at, title, text, raw_text, language, model, style, duration_ms, words, favorite, audio_path";

pub fn insert(conn: &Connection, t: &NewTranscript) -> rusqlite::Result<i64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let words = vaimozhi_core::count_words(t.text) as i64;
    conn.execute(
        "INSERT INTO transcripts(created_at, title, text, raw_text, language, model, style, duration_ms, words, audio_path)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![now, t.title, t.text, t.raw_text, t.language, t.model, t.style, t.duration_ms, words, t.audio_path],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Option<Transcript> {
    conn.query_row(
        &format!("SELECT {COLS} FROM transcripts WHERE id = ?1"),
        params![id],
        row_to_transcript,
    )
    .optional()
    .ok()
    .flatten()
}

pub fn audio_path(conn: &Connection, id: i64) -> Option<String> {
    conn.query_row("SELECT audio_path FROM transcripts WHERE id = ?1", params![id], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
        .flatten()
}

pub fn list(
    conn: &Connection,
    query: Option<&str>,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    favorites_only: bool,
    limit: i64,
) -> rusqlite::Result<Vec<Transcript>> {
    let mut sql = format!("SELECT {COLS} FROM transcripts WHERE 1=1");
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) {
        let escaped = q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        sql.push_str(" AND (text LIKE ? ESCAPE '\\' OR title LIKE ? ESCAPE '\\')");
        let pat = format!("%{}%", escaped);
        args.push(pat.clone().into());
        args.push(pat.into());
    }
    if let Some(f) = from_ms {
        sql.push_str(" AND created_at >= ?");
        args.push(f.into());
    }
    if let Some(t) = to_ms {
        sql.push_str(" AND created_at <= ?");
        args.push(t.into());
    }
    if favorites_only {
        sql.push_str(" AND favorite = 1");
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT ?");
    args.push(limit.into());
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), row_to_transcript)?;
    rows.collect()
}

pub fn update_text(conn: &Connection, id: i64, text: &str) -> rusqlite::Result<()> {
    let words = vaimozhi_core::count_words(text) as i64;
    conn.execute("UPDATE transcripts SET text = ?1, words = ?2 WHERE id = ?3", params![text, words, id])?;
    Ok(())
}

pub fn rename(conn: &Connection, id: i64, title: &str) -> rusqlite::Result<()> {
    conn.execute("UPDATE transcripts SET title = ?1 WHERE id = ?2", params![title, id])?;
    Ok(())
}

pub fn set_favorite(conn: &Connection, id: i64, fav: bool) -> rusqlite::Result<()> {
    conn.execute("UPDATE transcripts SET favorite = ?1 WHERE id = ?2", params![fav as i64, id])?;
    Ok(())
}

/// Delete one transcript; returns its audio path so the caller can remove the file.
pub fn delete(conn: &Connection, id: i64) -> rusqlite::Result<Option<String>> {
    let audio = audio_path(conn, id);
    conn.execute("DELETE FROM transcripts WHERE id = ?1", params![id])?;
    Ok(audio)
}

/// Delete everything; returns every audio path that existed.
pub fn clear(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT audio_path FROM transcripts WHERE audio_path IS NOT NULL")?;
    let paths: Vec<String> = stmt.query_map([], |r| r.get(0))?.filter_map(Result::ok).collect();
    drop(stmt);
    conn.execute("DELETE FROM transcripts", [])?;
    Ok(paths)
}

/// (total words, transcript count)
pub fn stats(conn: &Connection) -> (i64, i64) {
    conn.query_row("SELECT COALESCE(SUM(words), 0), COUNT(*) FROM transcripts", [], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })
    .unwrap_or((0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("vaimozhi-test-{}-{}.db", std::process::id(), n));
        let _ = std::fs::remove_file(&path);
        open(&path).unwrap()
    }

    fn add(conn: &Connection, text: &str) -> i64 {
        insert(
            conn,
            &NewTranscript {
                title: "",
                text,
                raw_text: text,
                language: "english",
                model: "small",
                style: "clean",
                duration_ms: 1000,
                audio_path: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn insert_search_favorite_delete() {
        let c = mem();
        let a = add(&c, "hello 100% world");
        let _b = add(&c, "வணக்கம் நண்பா");
        assert_eq!(list(&c, Some("100%"), None, None, false, 10).unwrap().len(), 1);
        assert_eq!(list(&c, Some("வணக்கம்"), None, None, false, 10).unwrap().len(), 1);
        set_favorite(&c, a, true).unwrap();
        assert_eq!(list(&c, None, None, None, true, 10).unwrap().len(), 1);
        assert_eq!(stats(&c), (5, 2));
        delete(&c, a).unwrap();
        assert_eq!(stats(&c).1, 1);
        clear(&c).unwrap();
        assert_eq!(stats(&c), (0, 0));
    }

    #[test]
    fn kv_roundtrip() {
        let c = mem();
        assert_eq!(kv_get(&c, "x"), None);
        kv_set(&c, "x", "1").unwrap();
        kv_set(&c, "x", "2").unwrap();
        assert_eq!(kv_get(&c, "x").as_deref(), Some("2"));
    }
}
