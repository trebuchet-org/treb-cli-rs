PRD written to `ralph/tasks/prd-phase-6-Sender System.md`. Here's a summary of what it covers:

**5 User Stories:**

1. **US-001** — Add `foundry-wallets` dependency and scaffold the `sender.rs` module with `ResolvedSender` enum and resolution function signatures
2. **US-002** — Implement PrivateKey, Ledger, and Trezor sender resolution (mapping `SenderConfig` → `WalletSigner`)
3. **US-003** — InMemory test signers (anvil default accounts) and Safe/Governor stub variants with recursive sub-sender resolution
4. **US-004** — Wire resolved senders into `ScriptArgs.wallets` via `ScriptConfig` so forge's execution pipeline can sign transactions
5. **US-005** — Integration tests and error handling (circular references, address mismatches, invalid keys, multi-sender configs)

**Key design decisions:**
- `ResolvedSender` enum wraps `WalletSigner` for signable types and provides `Safe`/`Governor` stub variants with recursive sub-sender references
- Extends the existing `ScriptConfig` builder rather than creating a parallel path
- No CLI wallet flags — sender selection comes exclusively from treb config
- Safe/Governor are structural stubs only; full signing deferred to Phase 17
- Circular reference detection via visited-set during recursive resolution
