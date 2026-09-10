//! Compile-level checks of the facade surface: every documented name
//! is reachable through `bffi::`, the macros resolve through it (the
//! default facade paths, and the accepted-but-identical
//! `crate = "bffi"` opt-in), the namespaced modules resolve, and the
//! zero-copy view types live only behind the unsafe_zero_copy door.
#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(missing_docs)]

#[test]
fn core_and_error_names_are_reexported() {
    use bffi::JsErrorExt as _;

    let _: bffi::Handle = bffi::Handle::NULL;
    let _: bffi::TypeTag = bffi::TypeTag(0x8100);
    let _: bffi::ErrorCode = bffi::ErrorCode::Ok;
    let registry: &bffi::Registry = bffi::Registry::global();
    let _: Vec<bffi::TypeTag> = registry.declared_tags();
    let _: bffi::BffiError = bffi::BffiError::new(bffi::ErrorCode::Error, "probe");
    let _: Option<bffi::BffiError> = bffi::take_last_error();

    let _: bffi::JsErrorName = bffi::js_error_name(bffi::ErrorCode::InvalidUtf8);
    let _: bffi::JsErrorShape = bffi::ErrorCode::InvalidUtf8.to_js_shape();
    let _: Option<bffi::JsErrorShape> = bffi::take_last_error_shape();
}

#[test]
fn types_and_object_names_are_reexported() {
    let _: bffi::JsNumber = bffi::JsNumber::new(1.0);
    let _: bffi::CopiedBuf = bffi::CopiedBuf::from_slice(b"x");
    let _: Vec<u8> = bffi::string_to_bytes("x");

    // The zero-copy door: constructors at the root, view types only
    // behind the module.
    let bytes = b"hello".to_vec();
    let view = bffi::str_view(&bytes).expect("valid utf-8");
    let typed: bffi::unsafe_zero_copy::ZeroCopyStr<'_> = view;
    assert_eq!(typed.as_str(), "hello");
    let buf: bffi::unsafe_zero_copy::ZeroCopyBuf<'_> = bffi::buf_view(&bytes);
    assert_eq!(buf.as_slice(), b"hello");

    let _: u16 = bffi::TAG_MIN;
    let _: bool = bffi::tag_in_range(bffi::TypeTag(bffi::TAG_MIN));

    const SESSION: bffi::TypeTag = bffi::TypeTag(0x0170);
    static WRAP: std::sync::OnceLock<bffi::ObjectWrap<u32>> = std::sync::OnceLock::new();
    let wrap = WRAP.get_or_init(|| bffi::ObjectWrap::new(SESSION).expect("tag claimed once"));
    let handle = wrap.wrap(42).expect("room");
    assert_eq!(*wrap.get(handle).expect("live"), 42);
    wrap.release(handle).expect("released");
}

#[test]
fn callback_and_event_loop_names_are_reexported() {
    let handle = bffi::register(
        bffi::CallbackSig::new(bffi::ValueType::I32, &[]),
        std::sync::Arc::new(|_| bffi::Value::I32(1)),
    )
    .expect("callback table has room");
    assert_eq!(
        bffi::invoke(handle, &[]).expect("gated"),
        bffi::Value::I32(1)
    );
    assert!(bffi::revoke(handle));

    assert_eq!(
        bffi::pump(),
        0,
        "empty queue: the non-blocking drain executes nothing"
    );
    assert!(!bffi::is_running(), "no runner in this test");
    assert_eq!(bffi::pending(), 0);
    let _executed: u64 = bffi::executed_total();
}

#[test]
fn dts_names_are_reexported() {
    let module = bffi::ModuleDef {
        name: "probe",
        fns: &[bffi::FunctionDef {
            js_name: "add",
            export_name: "bffi_add",
            docs: &[],
            params: &[bffi::ParamDef {
                name: "a",
                ty: bffi::TsType::Number,
            }],
            ret: bffi::TsType::Number,
            abi: bffi::AbiSig {
                params: &[bffi::AbiType::U32],
                out: Some(bffi::AbiOut::Prim(bffi::AbiPrim::U32)),
            },
        }],
        classes: &[bffi::ClassDef {
            js_name: "point",
            release_export: "bffi_point_release",
            docs: &[],
            constructor: bffi::MethodDef {
                js_name: "constructor",
                export_name: "bffi_point_new",
                docs: &[],
                params: &[],
                ret: bffi::TsType::BigInt,
                abi: bffi::AbiSig {
                    params: &[],
                    out: Some(bffi::AbiOut::Handle),
                },
            },
            fields: &[bffi::FieldDef {
                js_name: "x",
                export_name: "bffi_point_x_get",
                docs: &[],
                ty: bffi::TsType::Number,
                out: bffi::AbiOut::Prim(bffi::AbiPrim::U32),
            }],
            methods: &[],
        }],
    };
    let rendered = bffi::render(&module);
    assert!(rendered.contains("export function add(a: number): number;"));
    assert!(rendered.contains("export class point {"));
    let sanitized = bffi::sanitize("class");
    assert_eq!(sanitized, "_class");
}

// The proc macros must resolve through the facade: a plain function
// annotated with `#[bffi]` - no attribute options - names the facade
// namespaces `::bffi::{core, types, dts, build}` by default.
#[bffi::bffi]
fn facade_probe(a: u32) -> u32 {
    a + 1
}

#[test]
fn bffi_attribute_resolves_through_the_facade() {
    let mut out = 0_u32;
    assert_eq!(bffi_facade_probe(1, &mut out), bffi::ErrorCode::Ok);
    assert_eq!(out, 2);
}

// Compatibility: `crate = "bffi"` must keep working and select
// exactly the same roots as the default.
#[bffi::bffi(crate = "bffi")]
fn facade_only_probe(a: u32) -> u32 {
    a * 3
}

#[test]
fn bffi_attribute_crate_option_resolves_through_the_facade() {
    let mut out = 0_u32;
    assert_eq!(bffi_facade_only_probe(7, &mut out), bffi::ErrorCode::Ok);
    assert_eq!(out, 21);
}

#[test]
fn namespace_modules_resolve() {
    let _: bffi::core::ErrorCode = bffi::core::ErrorCode::Ok;
    let _: bffi::core::Handle = bffi::core::Handle::NULL;

    let copied: bffi::types::CopiedBuf = bffi::types::CopiedBuf::from_slice(b"ns");
    assert_eq!(copied.as_slice(), b"ns");

    let _: bffi::dts::TsType = bffi::dts::TsType::Number;

    let _: u16 = bffi::object::TAG_MIN;
    assert!(bffi::object::tag_in_range(bffi::core::TypeTag(
        bffi::object::TAG_MIN
    )));

    // The async namespace is a raw identifier (`async` is a keyword);
    // the `#[bffi_async]` shims name `::bffi::r#async::...`.
    let _: bffi::r#async::AsyncValue = bffi::r#async::AsyncValue::Unit;
    let _: bffi::r#async::AsyncError = bffi::r#async::AsyncError::InvalidHandle;

    // The buffer path the facade-mode shims take: store + free.
    let handle =
        bffi::build::runtime::store_bytes(bffi::types::CopiedBuf::from_slice(b"roundtrip"))
            .expect("stored");
    assert!(bffi::build::runtime::free_buffer(handle));
}
