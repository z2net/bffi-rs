//! The static intermediate representation (IR) of TypeScript
//! declarations.
//!
//! Descriptors are plain `Copy` values over `&'static` slices so the
//! `#[bffi]` / `#[bffi_class]` macros can emit them as constants and
//! the renderer can consume them without allocation.
//!
//! # Canonical Rust -> TsType table
//!
//! The macro-side classification mirrors this table (see also the
//! type matrix in the `bffi-macros` README):
//!
//! | Rust                                   | TsType      |
//! |----------------------------------------|-------------|
//! | `i8` `i16` `i32` `u8` `u16` `u32` `f32` `f64` | `Number` |
//! | `i64` `u64` (including handles)        | `BigInt`    |
//! | `bool`                                 | `Boolean`   |
//! | `&str`, `String`                       | `String`    |
//! | `Vec<u8>`, `CopiedBuf`                 | `Uint8Array`|
//! | `Option<String>`                       | `NullableString` |
//! | `Option<Vec<u8>>`, `Option<CopiedBuf>` | `NullableUint8Array` |
//! | `()`                                   | `Void`      |
//!
//! The `Nullable*` variants render with `| null` (e.g.
//! `"string | null"`), so the nullability of an `Option` return is
//! part of the type contract itself. They are flat (payload-free)
//! variants on purpose: the IR must stay `Copy` so the macros can
//! emit descriptors as constants (`Box::new` is not allowed in const
//! context).
//!
//! The same flatness rule produced the `Promise*` variants
//! (`PromiseVoid`/`Number`/`BigInt`/`Boolean`/`String`/`Uint8Array`),
//! emitted by `#[bffi_async]`: an async export returns a task handle
//! at the ABI level and a `Promise<T>` at the JS level - the
//! descriptor describes the JS-level contract.
//!
//! # B1 composites
//!
//! Records (`#[derive(BffiRecord)]` structs) and unit enums
//! (`#[derive(BffiEnum)]`) are **named** types: [`TsType::Record`]
//! and [`TsType::Enum`] carry the declared name, and the shape lives
//! in the [`RecordDef`]/[`EnumDef`] tables on [`ModuleDef`]. Names
//! are `&'static str`, so the IR stays `Copy` and const-emittable.
//! Sequences (`Vec<T>` with a non-`u8` item) are flat variants:
//! [`TsType::NumberArray`] and friends render `T[]` and need no
//! table entry; [`TsType::RecordArray`] names its element record.

use std::borrow::Cow;

/// A TypeScript type referenced by a declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TsType {
    /// The TypeScript `number` type.
    Number,
    /// The TypeScript `bigint` type.
    BigInt,
    /// The TypeScript `boolean` type.
    Boolean,
    /// The TypeScript `string` type.
    String,
    /// The TypeScript `Uint8Array` type.
    Uint8Array,
    /// The TypeScript `string | null` type (`Option<String>` returns).
    NullableString,
    /// The TypeScript `Uint8Array | null` type (`Option<Vec<u8>>` /
    /// `Option<CopiedBuf>` returns).
    NullableUint8Array,
    /// The TypeScript `Promise<void>` type (`#[bffi_async]` exports).
    PromiseVoid,
    /// The TypeScript `Promise<number>` type (`#[bffi_async]` returns
    /// of the number-ish primitives).
    PromiseNumber,
    /// The TypeScript `Promise<bigint>` type (`#[bffi_async]` returns
    /// of `i64`/`u64`).
    PromiseBigInt,
    /// The TypeScript `Promise<boolean>` type (`#[bffi_async]` returns
    /// of `bool`).
    PromiseBoolean,
    /// The TypeScript `Promise<string>` type (`#[bffi_async]` returns
    /// of `String`).
    PromiseString,
    /// The TypeScript `Promise<Uint8Array>` type (`#[bffi_async]`
    /// returns of `Vec<u8>` / `CopiedBuf`).
    PromiseUint8Array,
    /// The TypeScript `void` type.
    Void,
    /// A named record type, declared in [`ModuleDef::records`] (the
    /// `#[derive(BffiRecord)]` table entry of the same name).
    Record(&'static str),
    /// A named unit-enum union, declared in [`ModuleDef::enums`] (the
    /// `#[derive(BffiEnum)]` table entry of the same name).
    Enum(&'static str),
    /// The TypeScript `number[]` type (`Vec` of number-ish items).
    NumberArray,
    /// The TypeScript `bigint[]` type (`Vec<i64>` / `Vec<u64>`).
    BigIntArray,
    /// The TypeScript `boolean[]` type (`Vec<bool>`).
    BooleanArray,
    /// The TypeScript `string[]` type (`Vec<String>`).
    StringArray,
    /// The TypeScript `<name>[]` type (`Vec` of a named record).
    RecordArray(&'static str),
}

impl TsType {
    /// The TypeScript name of this type, as written in a `.d.ts`
    /// file (e.g. `"number"`, `"Uint8Array"`, `"void"`). Composite
    /// names (`RecordArray`) are built on the fly, hence the `Cow`.
    #[must_use]
    pub fn as_str(&self) -> Cow<'static, str> {
        match self {
            Self::Number => Cow::Borrowed("number"),
            Self::BigInt => Cow::Borrowed("bigint"),
            Self::Boolean => Cow::Borrowed("boolean"),
            Self::String => Cow::Borrowed("string"),
            Self::Uint8Array => Cow::Borrowed("Uint8Array"),
            Self::NullableString => Cow::Borrowed("string | null"),
            Self::NullableUint8Array => Cow::Borrowed("Uint8Array | null"),
            Self::PromiseVoid => Cow::Borrowed("Promise<void>"),
            Self::PromiseNumber => Cow::Borrowed("Promise<number>"),
            Self::PromiseBigInt => Cow::Borrowed("Promise<bigint>"),
            Self::PromiseBoolean => Cow::Borrowed("Promise<boolean>"),
            Self::PromiseString => Cow::Borrowed("Promise<string>"),
            Self::PromiseUint8Array => Cow::Borrowed("Promise<Uint8Array>"),
            Self::Void => Cow::Borrowed("void"),
            Self::Record(name) => Cow::Borrowed(*name),
            Self::Enum(name) => Cow::Borrowed(*name),
            Self::NumberArray => Cow::Borrowed("number[]"),
            Self::BigIntArray => Cow::Borrowed("bigint[]"),
            Self::BooleanArray => Cow::Borrowed("boolean[]"),
            Self::StringArray => Cow::Borrowed("string[]"),
            Self::RecordArray(name) => Cow::Owned(format!("{name}[]")),
        }
    }
}

/// The exact C ABI width of an out-parameter slot.
///
/// Out-parameters carry primitives and 64-bit integers at their Rust
/// width plus `bool` (one byte on the C ABI); every byte-carrying
/// return (`String` / `Vec<u8>` / `CopiedBuf` / `Option` of those)
/// and every async task handle travels as a `u64` handle instead
/// ([`AbiOut::Handle`], CALLING-CONVENTION.md §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AbiPrim {
    /// `i8`
    I8,
    /// `i16`
    I16,
    /// `i32`
    I32,
    /// `u8`
    U8,
    /// `u16`
    U16,
    /// `u32`
    U32,
    /// `f32`
    F32,
    /// `f64`
    F64,
    /// `bool` (one byte on the C ABI; bun:ffi spells it `"u8"`).
    Bool,
    /// `i64` (JS sees `bigint`).
    I64,
    /// `u64` (JS sees `bigint`).
    U64,
}

impl AbiPrim {
    /// The canonical name of this width, as written in the loader
    /// JSON schema (e.g. `"u32"`, `"bool"`). The `bun:ffi` spelling
    /// (where it differs) is a runtime concern: only `bool` does
    /// (`"u8"` on the dlopen side).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Bool => "bool",
            Self::I64 => "i64",
            Self::U64 => "u64",
        }
    }
}

/// The exact C ABI shape of one parameter entry.
///
/// One entry per JS-visible parameter: a borrowed `&[u8]` is ONE
/// [`AbiType::PtrLen`] here even though it crosses the dlopen
/// declaration as a `("ptr", "u64")` pair - expanding the pair is the
/// loader's job, not the IR's. `bool` keeps its canonical name; the
/// `bun:ffi` spelling (`"u8"`) is applied by the runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AbiType {
    /// `i8` by value.
    I8,
    /// `i16` by value.
    I16,
    /// `i32` by value.
    I32,
    /// `u8` by value.
    U8,
    /// `u16` by value.
    U16,
    /// `u32` by value.
    U32,
    /// `f32` by value.
    F32,
    /// `f64` by value.
    F64,
    /// `i64` (JS sees `bigint`).
    I64,
    /// `u64` (JS sees `bigint`; handles travel as `u64` too).
    U64,
    /// `bool` (one byte; bun:ffi `"u8"`, JS coerces `0`/`1`).
    Bool,
    /// A NUL-terminated UTF-8 cstring pointer (borrowed `&str`).
    Cstring,
    /// A borrowed byte view: one entry here, a `("ptr", "u64")`
    /// dlopen pair after expansion (borrowed `&[u8]`).
    PtrLen,
}

impl AbiType {
    /// The canonical name of this shape, as written in the loader
    /// JSON schema (e.g. `"u32"`, `"cstring"`, `"ptr_len"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::I64 => "i64",
            Self::U64 => "u64",
            Self::Bool => "bool",
            Self::Cstring => "cstring",
            Self::PtrLen => "ptr_len",
        }
    }
}

/// The out-parameter description of an ABI signature: the slot the
/// shim writes the return value into (`*mut T` on the C ABI, declared
/// as a `"pointer"` dlopen argument and read through the TypedArray
/// of the recorded width).
///
/// `None` (on [`AbiSig::out`]) means the unit return: no slot at all.
/// [`AbiOut::Handle`] is the shared `u64` slot for every byte-carrying
/// return (`String` / `Vec<u8>` / `CopiedBuf` / `Option` of those),
/// every `#[bffi_async]` task handle, and every class constructor
/// (CALLING-CONVENTION.md §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AbiOut {
    /// A primitive slot at the exact width (including `bool` as one
    /// byte and the 64-bit integers).
    Prim(AbiPrim),
    /// The shared `u64` handle slot (transient buffers, async task
    /// handles, class instances).
    Handle,
}

impl AbiOut {
    /// The canonical name of this slot, as written in the loader JSON
    /// schema: the [`AbiPrim`] name or `"handle"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prim(prim) => prim.as_str(),
            Self::Handle => "handle",
        }
    }
}

/// The exact C ABI signature behind one descriptor: one
/// [`AbiType`] per JS-visible parameter (declaration order) plus the
/// out-parameter slot.
///
/// The signature describes the SHIM side of the boundary; the
/// receiver handle of class methods and getters is not part of
/// `params` (the loader always prepends it as a `u64` argument), and
/// the C ABI return value - always the `ErrorCode` - is not part of
/// `out` (CALLING-CONVENTION.md §1/§4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AbiSig {
    /// The parameter shapes, in declaration order.
    pub params: &'static [AbiType],
    /// The out-parameter slot; `None` for the unit return.
    pub out: Option<AbiOut>,
}

/// A single function parameter: its JS-visible name and type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamDef {
    /// The parameter name as it appears in the generated declaration.
    pub name: &'static str,
    /// The parameter type.
    pub ty: TsType,
}

/// A native function exposed to JavaScript.
///
/// `export_name` follows the `bffi_` + `js_name` convention and is
/// consumed by `bffi-build` to link the C ABI symbol; it is never
/// rendered into the `.d.ts` output. `docs` feeds the JSDoc block.
/// `abi` records the exact C ABI signature (loader-side concern; the
/// `.d.ts` renderer ignores it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionDef {
    /// The function name as seen from JavaScript.
    pub js_name: &'static str,
    /// The C ABI export symbol (`bffi_` + `js_name`); build-side only.
    pub export_name: &'static str,
    /// Doc comment lines, rendered as a JSDoc block.
    pub docs: &'static [&'static str],
    /// The parameters, in declaration order.
    pub params: &'static [ParamDef],
    /// The return type.
    pub ret: TsType,
    /// The exact C ABI signature behind the generated shim.
    pub abi: AbiSig,
}

/// A named module grouping the native functions, classes and B1
/// composite types it exports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleDef {
    /// The module name as seen from JavaScript.
    pub name: &'static str,
    /// The functions exported by this module.
    pub fns: &'static [FunctionDef],
    /// The classes exported by this module.
    pub classes: &'static [ClassDef],
    /// The record types (`#[derive(BffiRecord)]`) exported by this
    /// module, in declaration order.
    pub records: &'static [RecordDef],
    /// The unit enums (`#[derive(BffiEnum)]`) exported by this
    /// module, in declaration order.
    pub enums: &'static [EnumDef],
}

/// One field of a record: its JS-visible name, type and docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordFieldDef {
    /// The field name as it appears in the generated interface.
    pub name: &'static str,
    /// Doc comment lines, rendered as a JSDoc block.
    pub docs: &'static [&'static str],
    /// The field type.
    pub ty: TsType,
}

/// A record type crossing the boundary as a wire-encoded value: the
/// TS side sees an `interface`, the ABI side a transient buffer
/// (`Vec<u8>` of the wire encoding).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordDef {
    /// The record name as seen from JavaScript (the identifier used
    /// by [`TsType::Record`]).
    pub js_name: &'static str,
    /// Doc comment lines, rendered as a JSDoc block.
    pub docs: &'static [&'static str],
    /// The fields, in declaration order (the wire encoding is
    /// positional and must match this order).
    pub fields: &'static [RecordFieldDef],
}

/// One variant of a unit enum: just its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnumVariantDef {
    /// The variant name as it appears in the generated union
    /// (wire-encoded as a string of the same spelling).
    pub name: &'static str,
    /// Doc comment lines, rendered as a JSDoc block.
    pub docs: &'static [&'static str],
}

/// A unit enum crossing the boundary as its variant name: the TS
/// side sees a union of string literals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnumDef {
    /// The enum name as seen from JavaScript (the identifier used by
    /// [`TsType::Enum`]).
    pub js_name: &'static str,
    /// Doc comment lines, rendered as a JSDoc block.
    pub docs: &'static [&'static str],
    /// The variants, in declaration order.
    pub variants: &'static [EnumVariantDef],
}

/// A class constructor or method: the `FunctionDef` shape as seen
/// from inside a class body (`params` exclude the receiver; the
/// export symbol follows `bffi_<class>_<method>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MethodDef {
    /// The method name as seen from JavaScript; the constructor uses
    /// the literal `constructor`.
    pub js_name: &'static str,
    /// The C ABI export symbol; build-side only, never rendered.
    pub export_name: &'static str,
    /// Doc comment lines, rendered as a JSDoc block.
    pub docs: &'static [&'static str],
    /// The parameters, in declaration order.
    pub params: &'static [ParamDef],
    /// The return type.
    pub ret: TsType,
    /// The exact C ABI signature behind the generated shim
    /// (receiver excluded; the constructor writes the instance
    /// handle into an [`AbiOut::Handle`] slot).
    pub abi: AbiSig,
}

/// A class field exposed as a read-only getter (P2 v1: getters only -
/// the Arc-based ownership model has no safe setter).
///
/// The getter shim takes the instance handle and writes the field
/// into one primitive slot, recorded by `out`; the export symbol
/// follows the `bffi_<class>_<field>_get` convention and is carried
/// here so loader descriptors stay self-describing (no convention
/// re-derivation on the JS side).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldDef {
    /// The field name as seen from JavaScript.
    pub js_name: &'static str,
    /// The C ABI export symbol of the getter shim; build-side only.
    pub export_name: &'static str,
    /// Doc comment lines, rendered as a JSDoc block.
    pub docs: &'static [&'static str],
    /// The field type.
    pub ty: TsType,
    /// The getter's out-parameter slot.
    pub out: AbiOut,
}

/// A native class exposed to JavaScript: generated by
/// `#[bffi_class]` over an `ObjectWrap`-backed Rust struct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassDef {
    /// The class name as seen from JavaScript.
    pub js_name: &'static str,
    /// The C ABI export symbol of the generated release shim
    /// (`bffi_<name>_release`); build-side only, carried so loader
    /// descriptors stay self-describing (FinalizationRegistry +
    /// explicit `release()`).
    pub release_export: &'static str,
    /// Doc comment lines, rendered as a JSDoc block.
    pub docs: &'static [&'static str],
    /// The constructor declaration (exactly one in P2 v1).
    pub constructor: MethodDef,
    /// The read-only field getters, in declaration order.
    pub fields: &'static [FieldDef],
    /// The methods, in declaration order.
    pub methods: &'static [MethodDef],
}

#[cfg(test)]
mod tests {
    use super::{
        AbiOut, AbiPrim, AbiSig, AbiType, Cow, EnumDef, EnumVariantDef, FunctionDef, ModuleDef,
        ParamDef, RecordDef, RecordFieldDef, TsType,
    };

    const UNIT_ABI: AbiSig = AbiSig {
        params: &[],
        out: None,
    };
    const U32_ABI: AbiSig = AbiSig {
        params: &[],
        out: Some(AbiOut::Prim(AbiPrim::U32)),
    };
    const U64_ABI: AbiSig = AbiSig {
        params: &[],
        out: Some(AbiOut::Prim(AbiPrim::U64)),
    };
    const HANDLE_ABI: AbiSig = AbiSig {
        params: &[],
        out: Some(AbiOut::Handle),
    };

    #[test]
    fn ts_type_as_str_covers_all_variants() {
        assert_eq!(TsType::Number.as_str(), "number");
        assert_eq!(TsType::BigInt.as_str(), "bigint");
        assert_eq!(TsType::Boolean.as_str(), "boolean");
        assert_eq!(TsType::String.as_str(), "string");
        assert_eq!(TsType::Uint8Array.as_str(), "Uint8Array");
        assert_eq!(TsType::NullableString.as_str(), "string | null");
        assert_eq!(TsType::NullableUint8Array.as_str(), "Uint8Array | null");
        assert_eq!(TsType::Void.as_str(), "void");
    }

    #[test]
    fn abi_names_cover_all_variants() {
        assert_eq!(AbiPrim::I8.as_str(), "i8");
        assert_eq!(AbiPrim::I16.as_str(), "i16");
        assert_eq!(AbiPrim::I32.as_str(), "i32");
        assert_eq!(AbiPrim::U8.as_str(), "u8");
        assert_eq!(AbiPrim::U16.as_str(), "u16");
        assert_eq!(AbiPrim::U32.as_str(), "u32");
        assert_eq!(AbiPrim::F32.as_str(), "f32");
        assert_eq!(AbiPrim::F64.as_str(), "f64");
        assert_eq!(AbiPrim::Bool.as_str(), "bool");
        assert_eq!(AbiPrim::I64.as_str(), "i64");
        assert_eq!(AbiPrim::U64.as_str(), "u64");

        assert_eq!(AbiType::Cstring.as_str(), "cstring");
        assert_eq!(AbiType::PtrLen.as_str(), "ptr_len");
        assert_eq!(AbiType::Bool.as_str(), "bool");
        assert_eq!(AbiType::U64.as_str(), "u64");

        assert_eq!(AbiOut::Prim(AbiPrim::F64).as_str(), "f64");
        assert_eq!(AbiOut::Handle.as_str(), "handle");
    }

    #[test]
    fn abi_shapes_are_copy_and_compare_by_value() {
        fn assert_copy<T: Copy>() {}

        assert_copy::<AbiPrim>();
        assert_copy::<AbiType>();
        assert_copy::<AbiOut>();
        assert_copy::<AbiSig>();

        let sig = AbiSig {
            params: &[AbiType::Cstring],
            out: Some(AbiOut::Handle),
        };
        let same = AbiSig {
            params: &[AbiType::Cstring],
            out: Some(AbiOut::Handle),
        };
        let different_params = AbiSig {
            params: &[AbiType::PtrLen],
            out: Some(AbiOut::Handle),
        };
        let different_out = AbiSig {
            params: &[AbiType::Cstring],
            out: Some(AbiOut::Prim(AbiPrim::U64)),
        };
        assert_eq!(sig, same, "identical sigs must compare equal");
        assert_ne!(sig, different_params, "different params must not be equal");
        assert_ne!(sig, different_out, "different outs must not be equal");
    }

    #[test]
    fn param_def_compares_by_value() {
        let param = ParamDef {
            name: "value",
            ty: TsType::Number,
        };
        let same = ParamDef {
            name: "value",
            ty: TsType::Number,
        };
        let different_name = ParamDef {
            name: "other",
            ty: TsType::Number,
        };
        let different_ty = ParamDef {
            name: "value",
            ty: TsType::String,
        };
        assert_eq!(param, same, "identical params must compare equal");
        assert_ne!(param, different_name, "different names must not be equal");
        assert_ne!(param, different_ty, "different types must not be equal");
    }

    #[test]
    fn function_def_compares_by_value() {
        static DOCS: &[&str] = &["Adds two numbers."];
        static PARAMS: [ParamDef; 1] = [ParamDef {
            name: "a",
            ty: TsType::Number,
        }];
        static DIFFERENT_DOCS: &[&str] = &["Subtracts."];

        let function = FunctionDef {
            js_name: "add",
            export_name: "bffi_add",
            docs: DOCS,
            params: &PARAMS,
            ret: TsType::Number,
            abi: U32_ABI,
        };
        let same = FunctionDef {
            js_name: "add",
            export_name: "bffi_add",
            docs: DOCS,
            params: &PARAMS,
            ret: TsType::Number,
            abi: U32_ABI,
        };
        let different_js_name = FunctionDef {
            js_name: "sub",
            export_name: "bffi_add",
            docs: DOCS,
            params: &PARAMS,
            ret: TsType::Number,
            abi: U32_ABI,
        };
        let with_different_docs = FunctionDef {
            js_name: "add",
            export_name: "bffi_add",
            docs: DIFFERENT_DOCS,
            params: &PARAMS,
            ret: TsType::Number,
            abi: U32_ABI,
        };
        assert_eq!(function, same, "identical defs must compare equal");
        assert_ne!(
            function, different_js_name,
            "different js_name must not be equal"
        );
        assert_ne!(
            function, with_different_docs,
            "different docs must not be equal"
        );
    }

    #[test]
    fn module_def_compares_by_value() {
        static FNS_A: [FunctionDef; 1] = [FunctionDef {
            js_name: "add",
            export_name: "bffi_add",
            docs: &[],
            params: &[],
            ret: TsType::Number,
            abi: U32_ABI,
        }];
        static FNS_B: [FunctionDef; 1] = [FunctionDef {
            js_name: "sub",
            export_name: "bffi_sub",
            docs: &[],
            params: &[],
            ret: TsType::Void,
            abi: UNIT_ABI,
        }];

        let module = ModuleDef {
            name: "native",
            fns: &FNS_A,
            classes: &[],
            records: &[],
            enums: &[],
        };
        let same = ModuleDef {
            name: "native",
            fns: &FNS_A,
            classes: &[],
            records: &[],
            enums: &[],
        };
        let different = ModuleDef {
            name: "native",
            fns: &FNS_B,
            classes: &[],
            records: &[],
            enums: &[],
        };
        assert_eq!(module, same, "identical modules must compare equal");
        assert_ne!(module, different, "different fns must not be equal");
    }

    #[test]
    fn descriptors_are_copy_and_static() {
        fn assert_copy<T: Copy>() {}

        assert_copy::<TsType>();
        assert_copy::<ParamDef>();
        assert_copy::<FunctionDef>();
        assert_copy::<ModuleDef>();

        assert_eq!(FNS.len(), 2);
        assert_eq!(MODULE.fns.len(), 2);
        assert_eq!(MODULE.name, "native");
        assert_eq!(MODULE.fns[0].ret, TsType::BigInt);
        assert_eq!(MODULE.fns[1].ret, TsType::NullableUint8Array);
    }

    #[test]
    fn nullable_string_renders_with_null() {
        static NULLABLE_FNS: &[FunctionDef] = &[FunctionDef {
            js_name: "maybe_name",
            export_name: "bffi_maybe_name",
            docs: &[],
            params: &[],
            ret: TsType::NullableString,
            abi: HANDLE_ABI,
        }];
        let module = ModuleDef {
            name: "native",
            fns: NULLABLE_FNS,
            classes: &[],
            records: &[],
            enums: &[],
        };
        let rendered = crate::bffi_dts::render::render(&module);
        assert!(rendered.contains("export function maybe_name(): string | null;"));
    }

    #[test]
    fn b1_composite_names_render() {
        assert_eq!(TsType::Record("Point").as_str(), "Point");
        assert_eq!(TsType::Enum("Color").as_str(), "Color");
        assert_eq!(TsType::NumberArray.as_str(), "number[]");
        assert_eq!(TsType::BigIntArray.as_str(), "bigint[]");
        assert_eq!(TsType::BooleanArray.as_str(), "boolean[]");
        assert_eq!(TsType::StringArray.as_str(), "string[]");
        assert_eq!(
            TsType::RecordArray("Point").as_str(),
            Cow::<str>::Borrowed("Point[]")
        );
    }

    #[test]
    fn record_and_enum_defs_compare_by_value() {
        fn assert_copy<T: Copy>() {}

        assert_copy::<RecordFieldDef>();
        assert_copy::<RecordDef>();
        assert_copy::<EnumVariantDef>();
        assert_copy::<EnumDef>();

        static FIELDS: &[RecordFieldDef] = &[
            RecordFieldDef {
                name: "x",
                docs: &[],
                ty: TsType::Number,
            },
            RecordFieldDef {
                name: "label",
                docs: &[],
                ty: TsType::String,
            },
        ];
        static VARIANTS: &[EnumVariantDef] = &[EnumVariantDef {
            name: "Red",
            docs: &[],
        }];

        let record = RecordDef {
            js_name: "Point",
            docs: &[],
            fields: FIELDS,
        };
        assert_eq!(record.fields.len(), 2);
        assert_eq!(record.fields[1].name, "label");

        let enumeration = EnumDef {
            js_name: "Color",
            docs: &[],
            variants: VARIANTS,
        };
        assert_eq!(enumeration.variants.len(), 1);
        assert_eq!(enumeration.variants[0].name, "Red");
    }

    static FNS: &[FunctionDef] = &[
        FunctionDef {
            js_name: "tick",
            export_name: "bffi_tick",
            docs: &[],
            params: &[],
            ret: TsType::BigInt,
            abi: U64_ABI,
        },
        FunctionDef {
            js_name: "peek",
            export_name: "bffi_peek",
            docs: &[],
            params: &[],
            ret: TsType::NullableUint8Array,
            abi: HANDLE_ABI,
        },
    ];

    static MODULE: ModuleDef = ModuleDef {
        name: "native",
        fns: FNS,
        classes: &[],
        records: &[],
        enums: &[],
    };
}
