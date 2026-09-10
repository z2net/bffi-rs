//! Acceptance tests for the generated `ClassDef` descriptors
//! (kanboard P2: integration with bffi-object and bffi-dts): the
//! metadata split across the two macro expansions assembles into one
//! `ClassDef` that renders through `bffi-dts`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bffi::{AbiOut, AbiPrim, AbiSig, AbiType, ClassDef, MethodDef, ModuleDef, ParamDef, TsType};

#[bffi_macros::bffi_class(tag = 0x0160)]
/// A wallet.
pub struct Wallet {
    /// The balance.
    pub balance: u64,
}

#[bffi_macros::bffi_impl]
impl Wallet {
    #[bffi_macros::bffi_constructor]
    /// Creates a wallet.
    pub fn new(start: u64) -> Self {
        Self { balance: start }
    }

    /// Adds an amount.
    pub fn topped(&self, amount: u64) -> u64 {
        self.balance + amount
    }

    /// Finds a note.
    pub fn find_note(&self, hit: bool) -> Option<String> {
        if hit {
            Some(format!("balance {}", self.balance))
        } else {
            None
        }
    }
}

#[test]
fn class_descriptor_matches_the_expected_literal() {
    assert_eq!(bffi_meta_wallet::TAG, 0x0160);
    assert_eq!(bffi_meta_wallet::FIELDS.len(), 1);
    assert_eq!(
        bffi_meta_wallet::FIELDS[0],
        bffi::FieldDef {
            js_name: "balance",
            export_name: "bffi_wallet_balance_get",
            docs: &[],
            ty: TsType::BigInt,
            out: AbiOut::Prim(AbiPrim::U64),
        }
    );
    assert_eq!(
        bffi_meta_wallet_impl::CLASS,
        ClassDef {
            js_name: "wallet",
            release_export: "bffi_wallet_release",
            docs: &["A wallet."],
            constructor: MethodDef {
                js_name: "constructor",
                export_name: "bffi_wallet_new",
                docs: &["Creates a wallet."],
                params: &[ParamDef {
                    name: "start",
                    ty: TsType::BigInt,
                }],
                ret: TsType::BigInt,
                abi: AbiSig {
                    params: &[AbiType::U64],
                    out: Some(AbiOut::Handle),
                },
            },
            fields: &[bffi::FieldDef {
                js_name: "balance",
                export_name: "bffi_wallet_balance_get",
                docs: &[],
                ty: TsType::BigInt,
                out: AbiOut::Prim(AbiPrim::U64),
            }],
            methods: &[
                MethodDef {
                    js_name: "topped",
                    export_name: "bffi_wallet_topped",
                    docs: &["Adds an amount."],
                    params: &[ParamDef {
                        name: "amount",
                        ty: TsType::BigInt,
                    }],
                    ret: TsType::BigInt,
                    abi: AbiSig {
                        params: &[AbiType::U64],
                        out: Some(AbiOut::Prim(AbiPrim::U64)),
                    },
                },
                // `Option` method returns render honest `| null`
                // types; `docs` contain only the author lines (no
                // auto-generated notes). The payload travels through
                // the shared handle slot.
                MethodDef {
                    js_name: "find_note",
                    export_name: "bffi_wallet_find_note",
                    docs: &["Finds a note."],
                    params: &[ParamDef {
                        name: "hit",
                        ty: TsType::Boolean,
                    }],
                    ret: TsType::NullableString,
                    abi: AbiSig {
                        params: &[AbiType::Bool],
                        out: Some(AbiOut::Handle),
                    },
                },
            ],
        }
    );
}

#[test]
fn class_descriptor_renders_through_bffi_dts() {
    static CLASSES: &[ClassDef] = &[bffi_meta_wallet_impl::CLASS];
    let module = ModuleDef {
        name: "bank",
        fns: &[],
        classes: CLASSES,
        records: &[],
        enums: &[],
    };
    let rendered = bffi::render(&module);
    assert!(rendered.contains("/** A wallet. */"));
    assert!(rendered.contains("export class wallet {"));
    assert!(rendered.contains("  constructor(start: bigint);"));
    assert!(rendered.contains("  get balance(): bigint;"));
    assert!(rendered.contains("  topped(amount: bigint): bigint;"));
    assert!(rendered.contains("  find_note(hit: boolean): string | null;"));
    assert!(rendered.ends_with("}\n"));
}
