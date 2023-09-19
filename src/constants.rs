pub mod spl_token {
    use solana_sdk::declare_id;

    declare_id!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
}

pub mod associated_token {
    use solana_sdk::declare_id;

    declare_id!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
}

pub mod mints {
    pub mod bsol {
        use solana_sdk::declare_id;

        declare_id!("bSo13r4TkiE4KumL71LsHTPpL2euBYLFx6h9HP3piy1");
    }

    pub mod uxd {
        use solana_sdk::declare_id;

        declare_id!("7kbnvuGBxxj8AG9qp8Scn56muWGaRaFqxg1FsRp3PaFT");
    }

    pub mod usdt {
        use solana_sdk::declare_id;

        declare_id!("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB");
    }

    pub mod usdc {
        use solana_sdk::declare_id;

        declare_id!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
    }
}

pub mod marginfi {
    pub mod group {
        use solana_sdk::declare_id;

        declare_id!("4qp6Fx6tnZkY5Wropq9wUYgtFxXKwE6viZxFHg3rdAG8");
    }

    pub mod banks {
        pub mod bsol {
            use solana_sdk::declare_id;

            declare_id!("6hS9i46WyTq1KXcoa2Chas2Txh9TJAVr6n1t3tnrE23K");
        }

        pub mod uxd {
            use solana_sdk::declare_id;

            declare_id!("BeNBJrAh1tZg5sqgt8D6AWKJLD5KkBrfZvtcgd7EuiAR");
        }

        pub mod usdt {
            use solana_sdk::declare_id;

            declare_id!("HmpMfL8942u22htC4EMiWgLX931g3sacXFR6KjuLgKLV");
        }

        pub mod usdc {
            use solana_sdk::declare_id;

            declare_id!("4SryZ4bWGqEsNjbqNUKuxnoyagWgbxj6MavyUF2HRzhA");
        }
    }
}

pub mod meteora {
    pub mod acusd_usdc_pool {
        use solana_sdk::declare_id;

        declare_id!("6ZLKLjMd2KzH7PPHCXUPgbMAtdTT37VgTtdeXWLoJppr");
    }

    pub mod acusd_usdc_farm {
        use solana_sdk::declare_id;

        declare_id!("9dGX6N3FLAVfKmvtkwHA9MVGsvEqGKnLFDQQFbw5dprr");
    }

    pub mod farm {
        use solana_sdk::declare_id;

        declare_id!("FarmuwXPWXvefWUeqFAa5w6rifLkq5X6E8bimYvrhCB1");
    }
}
