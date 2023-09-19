# Marginfi <-> Meteora farmer

- Bot that farms incentives / points on marginfi and meteora

## How it works

- Deposit selected funds to marginfi
- Borrow funds up to 90% utilizations based on borrow rates
- Swap borrowed funds if needed to USDC
- Deposit USDC to meteora pools
  - UXD/USDC
  - acUSDC/USDC
- Claims rewards from meteora every 8 hours and repeats the process with claimed funds
