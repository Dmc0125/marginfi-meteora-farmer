# Disclaimer
## Important:

This software is provided for testing and educational purposes only. Utilizing this software as provided may result in financial loss. The creator(s) of this software bear no responsibility for any financial or other damages incurred.

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
