const COMMANDS: &[&str] = &["attach", "set_style", "animate_to", "fade", "set_regions", "cover_window", "set_scale", "detach", "supported"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
