//! Liste des jeux d'opentrack (`facetracknoir supported games.csv`) et tables TrackIR.
//!
//! Les jeux « TrackIR Enhanced » n'acceptent que des données chiffrées avec une table de 8 octets
//! propre à chacun. Les DLL lisent cette table dans `HKCU\Software\MousePods\Table\<id>` ; on y
//! écrit celle de tous les jeux de la liste, si bien que n'importe quel jeu reconnu fonctionne.
//! Découpage de la clé identique à opentrack (`csv/csv.cpp`, `do_scanf`).

use std::path::Path;

/// Liste fournie avec podtrack (copie de celle d'opentrack, licence ISC).
pub const BUNDLED_CSV: &str = include_str!("../data/opentrack-games.csv");

#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    pub name: String,
    pub id: u32,
    /// Table de chiffrement, None pour un jeu non chiffré.
    pub table: Option<[u8; 8]>,
}

/// 11 octets de la clé → table : b5 b4 b3 b2 b9 b8 b7 b6 (les autres sont ignorés).
pub fn table_from_key(key: &str) -> Option<[u8; 8]> {
    if key.len() != 22 || !key.is_ascii() {
        return None;
    }
    let mut b = [0u8; 11];
    for (i, o) in b.iter_mut().enumerate() {
        *o = u8::from_str_radix(&key[2 * i..2 * i + 2], 16).ok()?;
    }
    Some([b[5], b[4], b[3], b[2], b[9], b[8], b[7], b[6]])
}

pub fn parse(csv: &str) -> Vec<Game> {
    let mut out = Vec::new();
    for line in csv.lines().skip(1) {
        let col: Vec<&str> = line.trim_end_matches('\r').split(';').collect();
        if col.len() < 8 {
            continue;
        }
        let Ok(id) = col[6].trim().parse::<u32>() else { continue };
        // comme opentrack : pas de table pour les jeux du protocole V160
        let table = if col[3].trim() == "V160" { None } else { table_from_key(col[7].trim()) };
        out.push(Game { name: col[1].trim().to_string(), id, table: table.filter(|t| t.iter().any(|&c| c != 0)) });
    }
    out
}

pub fn load(path: Option<&Path>) -> Vec<Game> {
    match path.and_then(|p| std::fs::read_to_string(p).ok()) {
        Some(text) => parse(&text),
        None => parse(BUNDLED_CSV),
    }
}

/// Tables à écrire dans le registre d'un préfixe.
#[derive(Debug, Clone, Default)]
pub struct TrackTable {
    pub entries: Vec<(u32, [u8; 8])>,
}

impl TrackTable {
    pub fn from_games(games: &[Game]) -> Self {
        let mut entries: Vec<(u32, [u8; 8])> = Vec::new();
        for g in games {
            if let Some(t) = g.table {
                if !entries.iter().any(|e| e.0 == g.id) {
                    entries.push((g.id, t));
                }
            }
        }
        Self { entries }
    }

    pub fn reg_lines(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|(id, t)| format!("\"{id}\"=hex:{}", t.iter().map(|c| format!("{c:02x}")).collect::<Vec<_>>().join(",")))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enhanced_game_table() {
        // jeu « TrackIR Enhanced » de la liste, pris comme vecteur de contrôle
        let games = parse(BUNDLED_CSV);
        let sc = games.iter().find(|g| g.name == "Star Citizen").unwrap();
        assert_eq!(sc.id, 3450);
        assert_eq!(sc.table, Some([0x34, 0x4e, 0xce, 0xf4, 0xa2, 0xb4, 0xc6, 0x3e]));
        let t = TrackTable::from_games(&games);
        assert!(t.entries.len() >= 60, "{}", t.entries.len());
        assert!(t.reg_lines().contains(&"\"3450\"=hex:34,4e,ce,f4,a2,b4,c6,3e".to_string()));
    }
}
