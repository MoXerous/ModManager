//! Modpack support. Only the Modrinth .mrpack format exists (ADR 0003 §2:
//! CurseForge arrives in M3 as a new design, with no behaviour to port).

pub mod mrpack;

#[cfg(test)]
mod tests;
