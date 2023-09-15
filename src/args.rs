use clap::Parser;

const NAMESPACE: &'static str = "[CONFIG_ERROR]:";

pub fn load_arg(key: &str) -> String {
    std::env::var(key).expect(&format!("{NAMESPACE} Argument {key} is missing"))
}

pub fn load_and_parse_arg<T, F: Fn(String) -> Result<T, String>>(key: &str, parse_fn: F) -> T {
    parse_fn(load_arg(key)).expect(&format!("{NAMESPACE} Could not parse {key} argument"))
}

#[derive(Debug, Parser)]
pub struct CliArgs {
    #[arg(long = "bsol", default_value_t = 0)]
    bsol_amount: u64,

    #[arg(long = "uxd", default_value_t = 0)]
    uxd_amount: u64,
}
