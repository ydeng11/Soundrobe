// Prevents an additional console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--nominate-library")) {
        let root = args
            .next()
            .filter(|_| args.next().is_none())
            .ok_or("Usage: soundrobe --nominate-library <library-directory>")?;
        let home = dirs::home_dir().ok_or("Could not locate the Soundrobe profile")?;
        let profile = soundrobe_lib::state::paths::AppDataPaths::desktop(home);
        profile.prepare()?;
        let owner =
            soundrobe_lib::state::library_authority::load_or_create_profile_id(profile.data_dir())?;
        soundrobe_lib::state::library_authority::nominate(std::path::Path::new(&root), owner)?;
        return Ok(());
    }
    soundrobe_lib::run();
    Ok(())
}
