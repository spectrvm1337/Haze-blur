const COMMANDS: &[&str] = &["attach", "set_style", "animate_to", "fade", "set_regions", "detach"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
