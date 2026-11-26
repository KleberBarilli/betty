// Em release, não abre a janela de console junto com o app no Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    mybettys_lib::run();
}
