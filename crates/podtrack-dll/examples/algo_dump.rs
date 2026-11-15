//! Affiche somme de contrôle et chiffrement sur des tampons de référence (comparaison avec le C).
fn main() {
    let t = [0x34, 0x4e, 0xce, 0xf4, 0xa2, 0xb4, 0xc6, 0x3e];
    for n in 0..=80usize {
        let buf: Vec<u8> = (0..n).map(|i| ((i * 37 + 11) & 255) as u8).collect();
        let c = podtrack_dll::algo::cksum(&buf);
        let mut e = buf.clone();
        podtrack_dll::algo::enhance(&mut e, &t);
        println!("{n} {c:08x} {}", e.iter().map(|b| format!("{b:02x}")).collect::<String>());
    }
}
