//! Sonde Windows : charge NPClient64.dll et affiche ce que NP_GetData renvoie.
//! Usage (Wine) : np_probe.exe C:\podtrack\NPClient64.dll [id]
#[cfg(windows)]
fn main() {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryA(n: *const u8) -> *mut c_void;
        fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
        fn Sleep(ms: u32);
    }
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("cmp") {
        unsafe {
            let id: u16 = args[4].parse().unwrap();
            for dll in [&args[2], &args[3]] {
                let h = LoadLibraryA(format!("{dll}\0").as_ptr());
                let reg: extern "system" fn(u16) -> i32 = std::mem::transmute(GetProcAddress(h, b"NP_RegisterProgramProfileID\0".as_ptr()));
                let get: extern "system" fn(*mut u8) -> i32 = std::mem::transmute(GetProcAddress(h, b"NP_GetData\0".as_ptr()));
                reg(id);
                for _ in 0..3 {
                    let mut buf = [0u8; 68];
                    let r = get(buf.as_mut_ptr());
                    println!("{r} {}", buf.iter().map(|b| format!("{b:02x}")).collect::<String>());
                }
            }
        }
        return;
    }
    let path = format!("{}\0", args.get(1).map(String::as_str).unwrap_or("C:\\podtrack\\NPClient64.dll"));
    let id: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    unsafe {
        let h = LoadLibraryA(path.as_ptr());
        println!("LoadLibrary: {h:?}");
        if h.is_null() { return; }
        let reg: extern "system" fn(u16) -> i32 = std::mem::transmute(GetProcAddress(h, b"NP_RegisterProgramProfileID\0".as_ptr()));
        let get: extern "system" fn(*mut u8) -> i32 = std::mem::transmute(GetProcAddress(h, b"NP_GetData\0".as_ptr()));
        println!("register({id}) = {}", reg(id));
        if args.get(3).map(String::as_str) == Some("rate") {
            let t0 = std::time::Instant::now();
            let (mut n, mut changes, mut last, mut worst) = (0u32, 0u32, f32::NAN, 0u128);
            while t0.elapsed().as_secs() < 3 {
                let mut buf = [0u8; 68];
                let c = std::time::Instant::now();
                get(buf.as_mut_ptr());
                worst = worst.max(c.elapsed().as_micros());
                let yaw = f32::from_le_bytes(buf[16..20].try_into().unwrap());
                if yaw != last { changes += 1; last = yaw; }
                n += 1;
                Sleep(2);
            }
            println!("{n} appels en 3 s, {} valeurs de lacet distinctes/s, pire appel {worst} µs", changes / 3);
            return;
        }
        for _ in 0..10 {
            let mut buf = [0u8; 68];
            let r = get(buf.as_mut_ptr());
            let f = |o: usize| f32::from_le_bytes(buf[o..o + 4].try_into().unwrap());
            let st = i16::from_le_bytes([buf[0], buf[1]]);
            let fr = i16::from_le_bytes([buf[2], buf[3]]);
            println!("ret {r} status {st} frame {fr} roll {:.0} pitch {:.0} yaw {:.0}", f(8), f(12), f(16));
            Sleep(200);
        }
    }
}
#[cfg(not(windows))]
fn main() {}
