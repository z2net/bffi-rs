/** `bffi init [--root <dir>] [--module <m>] [--crate-dir <d>]
 * [--crate-name <n>] [--binary <b>]`: scaffolds `.bffi/bffi.json`
 * plus a minimal Rust crate (cdylib + emit-json). Refuses to
 * overwrite an existing config. */
import {
  CONFIG_VERSION,
  defineConfig,
  joinOut,
} from "@z2net/bffi";
import { flagString, parseArgs } from "../args.ts";
import { EXIT, writeErr, writeOut } from "../output.ts";

export const usage =
  `bffi init [--root <dir>] [--module <m>] [--crate-dir <d>] [--crate-name <n>] [--binary <b>]`;

export async function init(argv: string[]): Promise<number> {
  const args = parseArgs(argv);
  const root = flagString(args, "root") ?? process.cwd();
  const module = flagString(args, "module") ?? "native";
  const crateDir = flagString(args, "crate-dir") ?? module;
  const crateName = flagString(args, "crate-name") ?? `bffi-${module}`;
  const binary = flagString(args, "binary") ?? crateName.replaceAll("-", "_");

  const bffiDir = joinOut(root, ".bffi");
  const configPath = joinOut(bffiDir, "bffi.json");
  const configFile = Bun.file(configPath);
  if (await configFile.exists()) {
    writeErr(`init: config already exists: ${configPath} (not overwritten)`);
    return EXIT.fail;
  }

  const config = defineConfig({
    bffi: CONFIG_VERSION,
    version: "0.1.0",
    module,
    os: "auto",
    debug: false,
    bun: "bun",
    rust: "cargo",
    crate: { dir: crateDir, name: crateName, binary },
    generate: { apiGen: true, outFile: "api.gen.ts" },
    files: ["bffi.api.json", "api.gen.ts"],
    libraryPath: null,
  });
  await Bun.write(configPath, `${JSON.stringify(config, null, 2)}\n`);

  await scaffoldCrate(root, crateDir, crateName, binary);

  writeOut(`init: created ${configPath}`);
  writeOut("init: next - implement your crate surface, then: bun bffi build");
  return EXIT.ok;
}

/** Writes the minimal crate skeleton: Cargo.toml (cdylib), lib.rs
 * with one sample `#[bffi]` function, module_def.rs and the
 * emit-json bin. */
async function scaffoldCrate(
  root: string,
  crateDir: string,
  crateName: string,
  binary: string,
): Promise<void> {
  const crateRoot = joinOut(root, crateDir);
  const emitJsonName = `${binary}_emit_json`;
  const libIdent = crateName.replaceAll("-", "_");

  const cargoToml = `[package]
name = "${crateName}"
version = "0.1.0"
edition = "2021"
license = "MIT"

[lib]
crate-type = ["cdylib", "rlib"]

[[bin]]
name = "${emitJsonName}"
path = "src/bin/emit_json.rs"

[dependencies]
bffi = "0.1.2"
`;
  await Bun.write(joinOut(crateRoot, "Cargo.toml"), cargoToml);

  const libRs = `//! Sample crate scaffolded by \`bffi init\`. Replace the sample
//! function with your real surface, aggregate it in \`module_def\`,
//! and keep the emit-json bin (it writes .bffi/bffi.api.json).

pub mod module_def;

bffi::bffi_runtime_abi!();

/// Sample export: replace with your surface.
#[bffi::bffi]
pub fn hello() -> String {
    "hello from bffi".to_owned()
}
`;
  await Bun.write(joinOut(crateRoot, "src", "lib.rs"), libRs);

  const moduleDef = `use bffi::{FunctionDef, ModuleDef};

pub const FUNCTIONS: &[FunctionDef] = &[crate::bffi_meta_hello::FUNCTION];

pub const MODULE: ModuleDef = ModuleDef {
    name: "${moduleNameOf(crateName)}",
    fns: FUNCTIONS,
    classes: &[],
    records: &[],
    enums: &[],
    errors: &[],
};
`;
  await Bun.write(joinOut(crateRoot, "src", "module_def.rs"), moduleDef);

  const emitJson = `fn main() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".bffi/bffi.api.json");
    if let Err(error) =
        bffi::build::loader_json::write_to_file(&${libIdent}::module_def::MODULE, &path)
    {
        eprintln!("emit-json: {error}");
        std::process::exit(1);
    }
    println!("written {}", path.display());
}
`;
  await Bun.write(joinOut(crateRoot, "src", "bin", "emit_json.rs"), emitJson);

  // .gitignore for the crate: generated working files stay out of
  // git if the project chooses; the config itself is committed.
  const crateIgnore = `.bffi/bffi.api.json
.bffi/api.gen.ts
target/
`;
  await Bun.write(joinOut(crateRoot, ".gitignore"), crateIgnore);
}

/** Derives the module name from the crate name (dashes stripped). */
function moduleNameOf(crateName: string): string {
  return crateName.replaceAll("-", "_");
}
