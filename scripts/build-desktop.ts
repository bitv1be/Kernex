const environment = { ...process.env };

// linuxdeploy bundles an older strip binary that cannot read RELR sections
// emitted by current Linux distributions. Skipping its optional strip pass
// preserves compatibility while still producing the same application payload.
if (process.platform === "linux" && environment.NO_STRIP === undefined) {
  environment.NO_STRIP = "1";
}

const build = Bun.spawn(
  ["bun", "run", "--cwd", "crates/kernex-desktop", "tauri", "build"],
  { cwd: process.cwd(), env: environment, stdin: "inherit", stdout: "inherit", stderr: "inherit" },
);

process.exit(await build.exited);
