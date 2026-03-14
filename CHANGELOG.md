# Changelog

## [Release] v0.1.0 — Squelch Híbrido: Code Review & Refatoração

### O que foi modificado

- [x] **Backend (`crates/novasdr-server/src/ws/audio.rs`):** Eliminados magic numbers `10` — declarada constante `SQUELCH_HYSTERESIS_FRAMES: u8 = 10` e substituída em ambos os pontos de uso (modo manual e modo auto).
- [x] **Backend (`audio.rs`):** Removidos 3 comentários didáticos/decorativos que violavam as Engineering Rules (proibição de comentários óbvios).
- [x] **Documentação (`docs/AUDIO.md`):** Seção Squelch reescrita para documentar ambos os modos (Auto e Manual), incluindo payload `level: Option<f32>`, tabela de campos e constante de histerese.
- [x] **Frontend (`frontend/src/components/receiver/panels/AudioPanel.tsx`):** Slider do squelch alterado de opacity/pointer-events para **renderização condicional** — desaparece completamente quando `squelchAuto` é `true`.
- [x] **Frontend (`types.ts`, `useAudioClient.ts`):** Auditados e confirmados sem comentários inúteis (nenhuma modificação necessária).
