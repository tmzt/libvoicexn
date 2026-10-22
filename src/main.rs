//! The desktop entry: the app in libmorph's emulated phone.

fn main() -> Result<(), libmorph::MorphError> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    libmorph::run::<voicexn::App>()
}
