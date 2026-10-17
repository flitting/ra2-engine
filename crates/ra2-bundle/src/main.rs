//! Builds the game into a Mac app, and the app into a disk image to hand to
//! players:
//!
//! ```text
//! cargo run -p ra2-bundle                 # one app and image for Apple silicon, one for Intel
//! cargo run -p ra2-bundle -- --target apple-silicon   # just one (also: intel, or a Rust target triple)
//! cargo run -p ra2-bundle -- --native     # this Mac's kind only
//! cargo run -p ra2-bundle -- --sign "Developer ID Application: …" --notarize <profile>
//! cargo run -p ra2-bundle -- --sign … --notary-key AuthKey.p8 --notary-key-id … --notary-issuer …   # CI
//! ```
//!
//! The image holds the app, a link to /Applications to drag it onto, and a
//! note on opening it the first time. It holds none of the game's own files:
//! the app asks the player for their copy when it first runs (`setup.rs` in
//! the game), and the bundle is checked to hold only what is built here
//! before the image is made.
//!
//! Without `--sign` the app is signed ad hoc, which lets it run on the Mac it
//! was built on and, once the player has allowed it (see the note), on
//! others. `--sign` with a Developer ID and `--notarize` with a notarytool
//! keychain profile (`xcrun notarytool store-credentials`) make one that opens
//! with no warning anywhere.

use std::path::{Path, PathBuf};
use std::process::Command;

mod background;
mod icon;

/// The app's name as players see it.
const NAME: &str = "RA2 Engine";
/// The program inside the app (the game's binary).
const EXECUTABLE: &str = "ra2-rust";
const DEFAULT_ID: &str = "io.github.ra2-engine";
/// The kinds of Mac built for: Rust's target, the name in the image's file
/// name, and the oldest macOS it runs on (11 is the first for Apple silicon;
/// 10.13 the first with the graphics the game needs).
#[derive(Clone, Copy, PartialEq)]
struct Mac {
    target: &'static str,
    label: &'static str,
    minimum: &'static str,
}

const MACS: [Mac; 2] = [
    Mac { target: "aarch64-apple-darwin", label: "apple-silicon", minimum: "11.0" },
    Mac { target: "x86_64-apple-darwin", label: "intel", minimum: "10.13" },
];

/// A kind of Mac by its name, its architecture or its Rust target.
fn mac_named(name: &str) -> Option<Mac> {
    MACS.iter().copied().find(|mac| {
        mac.target == name || mac.label == name || (name == "arm64" && mac.label == "apple-silicon") || (name == "x86_64" && mac.label == "intel")
    })
}

/// How to log in to Apple's notary service: a profile saved in the keychain
/// with `notarytool store-credentials`, or an App Store Connect API key.
enum Notary {
    Profile(String),
    Key { path: String, id: String, issuer: String },
}

impl Notary {
    fn args(&self) -> Vec<String> {
        match self {
            Notary::Profile(profile) => vec!["--keychain-profile".into(), profile.clone()],
            Notary::Key { path, id, issuer } => vec!["--key".into(), path.clone(), "--key-id".into(), id.clone(), "--issuer".into(), issuer.clone()],
        }
    }
}

struct Options {
    /// Each is built, packed and imaged on its own.
    macs: Vec<Mac>,
    sign: Option<String>,
    notarize: Option<Notary>,
    /// The version in the image's name and the app's Info.plist; by default the
    /// package's with the commit.
    version: Option<String>,
    id: String,
    /// Skip building and use the program already built (for trying the
    /// packaging alone).
    no_build: bool,
    /// A disk image with no laid-out window (no Finder scripting needed).
    plain: bool,
}

fn main() {
    let options = match parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n\n{}", usage());
            std::process::exit(2);
        }
    };
    if let Err(message) = bundle(&options) {
        eprintln!("ra2-bundle: {message}");
        std::process::exit(1);
    }
}

fn usage() -> &'static str {
    "usage: cargo run -p ra2-bundle -- [--target apple-silicon|intel]... [--native] [--sign IDENTITY] [--notarize KEYCHAIN_PROFILE | --notary-key P8 --notary-key-id ID --notary-issuer ID] [--version V] [--id BUNDLE_ID] [--no-build] [--plain]"
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let (mut key, mut key_id, mut issuer) = (None, None, None);
    let mut options = Options { macs: Vec::new(), sign: None, notarize: None, version: None, id: DEFAULT_ID.into(), no_build: false, plain: false };
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--target" => {
                let name = value("--target")?;
                options.macs.push(mac_named(&name).ok_or(format!("unknown target {name}: apple-silicon, intel, or a Rust target"))?);
            }
            "--native" => options.macs.push(if cfg!(target_arch = "aarch64") { MACS[0] } else { MACS[1] }),
            "--sign" => options.sign = Some(value("--sign")?),
            "--notarize" => options.notarize = Some(Notary::Profile(value("--notarize")?)),
            "--notary-key" => key = Some(value("--notary-key")?),
            "--notary-key-id" => key_id = Some(value("--notary-key-id")?),
            "--notary-issuer" => issuer = Some(value("--notary-issuer")?),
            "--version" => options.version = Some(value("--version")?),
            "--id" => options.id = value("--id")?,
            "--no-build" => options.no_build = true,
            "--plain" => options.plain = true,
            "-h" | "--help" => {
                println!("{}", usage());
                std::process::exit(0);
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    if options.macs.is_empty() {
        options.macs = MACS.to_vec();
    }
    match (key, key_id, issuer) {
        (Some(path), Some(id), Some(issuer)) => options.notarize = Some(Notary::Key { path, id, issuer }),
        (None, None, None) => {}
        _ => return Err("--notary-key, --notary-key-id and --notary-issuer go together".into()),
    }
    if options.notarize.is_some() && options.sign.is_none() {
        return Err("--notarize needs --sign with a Developer ID: Apple only notarizes apps signed with one".into());
    }
    Ok(options)
}

/// Runs a command, failing with what it said if it fails.
fn run(command: &mut Command) -> Result<(), String> {
    let shown = format!("{command:?}");
    let status = command.status().map_err(|error| format!("could not run {shown}: {error}"))?;
    status.success().then_some(()).ok_or(format!("{shown} failed ({status})"))
}

/// A command's output as text.
fn output(command: &mut Command) -> Option<String> {
    let out = command.output().ok().filter(|out| out.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn bundle(options: &Options) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("a Mac app can only be built on a Mac".into());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().map_err(|error| error.to_string())?;
    let version = match &options.version {
        Some(version) => version.trim_start_matches('v').to_owned(),
        None => version(&root)?,
    };
    let out = root.join("target/bundle");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;

    for &mac in &options.macs {
        bundle_for(&root, &out, &version, mac, options)?;
    }
    Ok(())
}

/// The app and image for one kind of Mac, in `out/<label>/` and
/// `out/RA2-Engine-<version>-<label>.dmg`.
fn bundle_for(root: &Path, out: &Path, version: &str, mac: Mac, options: &Options) -> Result<(), String> {
    let work = out.join(mac.label);
    std::fs::create_dir_all(&work).map_err(|error| error.to_string())?;

    // The program.
    let program = build(root, mac, options.no_build)?;

    // The app.
    let app = work.join(format!("{NAME}.app"));
    let contents = app.join("Contents");
    for dir in ["MacOS", "Resources"] {
        std::fs::create_dir_all(contents.join(dir)).map_err(|error| error.to_string())?;
    }
    std::fs::copy(&program, contents.join("MacOS").join(EXECUTABLE)).map_err(|error| format!("copying the program: {error}"))?;
    std::fs::write(contents.join("Info.plist"), info_plist(&options.id, version, mac.minimum)).map_err(|error| error.to_string())?;
    std::fs::write(contents.join("PkgInfo"), "APPL????").map_err(|error| error.to_string())?;
    make_icon(&work, &contents.join("Resources/AppIcon.icns"))?;
    holds_only_ours(&app)?;
    sign(&app, options.sign.as_deref())?;
    // Notarized on its own first, with the ticket stapled to it, so the app
    // opens without a warning even on a Mac that is offline the first time
    // (a ticket only on the image would be looked up online for the app).
    if let Some(notary) = &options.notarize {
        let zip = work.join(format!("{NAME}.zip"));
        run(Command::new("ditto").args(["-c", "-k", "--keepParent"]).arg(&app).arg(&zip))?;
        notarize(&zip, notary)?;
        let _ = std::fs::remove_file(&zip);
        staple(&app)?;
    }
    println!("built {}", app.display());

    // The image.
    let stage = work.join("dmg");
    std::fs::create_dir_all(&stage).map_err(|error| error.to_string())?;
    run(Command::new("ditto").arg(&app).arg(stage.join(format!("{NAME}.app"))))?;
    std::os::unix::fs::symlink("/Applications", stage.join("Applications")).map_err(|error| error.to_string())?;
    std::fs::write(stage.join("Read Me First.txt"), read_me(version, options.notarize.is_some())).map_err(|error| error.to_string())?;
    let dmg = out.join(format!("RA2-Engine-{version}-{}.dmg", mac.label));
    if options.plain {
        make_image(&stage, &dmg, "UDZO")?;
    } else {
        laid_out_image(&work, &stage, &dmg)?;
    }
    let _ = std::fs::remove_dir_all(&stage);
    if let Some(identity) = &options.sign {
        run(Command::new("codesign").args(["--force", "--timestamp", "--sign", identity]).arg(&dmg))?;
    }
    if let Some(notary) = &options.notarize {
        notarize(&dmg, notary)?;
        staple(&dmg)?;
    }
    run(Command::new("hdiutil").args(["verify", "-quiet"]).arg(&dmg))?;
    let size = std::fs::metadata(&dmg).map(|meta| meta.len()).unwrap_or(0);
    println!("made {} ({:.1} MB)", dmg.display(), size as f64 / 1e6);
    Ok(())
}

/// Sends a file to Apple's notary service and waits for its answer; on a
/// rejection, prints Apple's log of why.
fn notarize(file: &Path, notary: &Notary) -> Result<(), String> {
    let out = Command::new("xcrun")
        .args(["notarytool", "submit", "--wait", "--output-format", "json"])
        .args(notary.args())
        .arg(file)
        .output()
        .map_err(|error| format!("could not run notarytool: {error}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() && text.contains("\"status\":\"Accepted\"") {
        println!("notarized {}", file.display());
        return Ok(());
    }
    let id = text.split("\"id\":\"").nth(1).and_then(|rest| rest.split('"').next()).unwrap_or("");
    if !id.is_empty() {
        let _ = Command::new("xcrun").args(["notarytool", "log", id]).args(notary.args()).status();
    }
    Err(format!("notarizing {} failed: {text}{}", file.display(), String::from_utf8_lossy(&out.stderr)))
}

/// Attaches the notary's ticket. Just after a notarization is accepted the
/// ticket may not be served yet (stapler's error 73), so it is tried for a
/// couple of minutes.
fn staple(file: &Path) -> Result<(), String> {
    for attempt in 1..=12 {
        let status = Command::new("xcrun").args(["stapler", "staple"]).arg(file).status().map_err(|error| error.to_string())?;
        if status.success() {
            return Ok(());
        }
        eprintln!("stapling {} failed (try {attempt} of 12); waiting for Apple's ticket", file.display());
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
    Err(format!("could not staple {}", file.display()))
}

fn make_image(stage: &Path, dmg: &Path, format: &str) -> Result<(), String> {
    run(Command::new("hdiutil")
        .args(["create", "-quiet", "-volname", NAME, "-fs", "HFS+", "-format", format, "-imagekey", "zlib-level=9", "-ov", "-srcfolder"])
        .arg(stage)
        .arg(dmg))
}

/// A disk image whose window opens on a picture with the app and the
/// Applications folder either side of an arrow (the usual way to say "drag to
/// install"). The window's layout lives in the volume's .DS_Store, which only
/// Finder writes, so the image is made writable, mounted, laid out by
/// scripting Finder, then compressed. The first time, macOS asks whether the
/// terminal may control Finder; without that the image is made plain.
fn laid_out_image(out: &Path, stage: &Path, dmg: &Path) -> Result<(), String> {
    // Mounted at a path of its own, so a copy already open (with the same
    // name) is no matter; Finder is pointed at it by that path.
    let mount = out.join("mount");
    let _ = std::fs::create_dir_all(&mount);
    let pictures = stage.join(".background");
    std::fs::create_dir_all(&pictures).map_err(|error| error.to_string())?;
    let (one, two) = (out.join("background.png"), out.join("background@2x.png"));
    background::draw(1, NAME).save(&one).map_err(|error| error.to_string())?;
    background::draw(2, NAME).save(&two).map_err(|error| error.to_string())?;
    // One file holding both sizes, so Retina screens get the sharp one.
    run(Command::new("tiffutil").arg("-cathidpicheck").arg(&one).arg(&two).arg("-out").arg(pictures.join("background.tiff")))?;
    let writable = out.join("writable.dmg");
    make_image(stage, &writable, "UDRW")?;
    run(Command::new("hdiutil").args(["attach", "-quiet", "-readwrite", "-noverify", "-noautoopen", "-mountpoint"]).arg(&mount).arg(&writable))?;
    let laid_out = run(Command::new("osascript").arg("-e").arg(layout_script(&mount)));
    let _ = Command::new("sync").status();
    let detached = run(Command::new("hdiutil").args(["detach", "-quiet"]).arg(&mount))
        .or_else(|_| run(Command::new("hdiutil").args(["detach", "-quiet", "-force"]).arg(&mount)));
    detached?;
    let _ = std::fs::remove_dir(&mount);
    if let Err(error) = laid_out {
        eprintln!("warning: couldn't lay out the window ({error}); the image will be plain. If macOS asked whether this terminal may control Finder, allow it (System Settings > Privacy & Security > Automation) and build again.");
        let _ = std::fs::remove_dir_all(&pictures);
        make_image(stage, dmg, "UDZO")?;
    } else {
        run(Command::new("hdiutil").args(["convert", "-quiet", "-format", "UDZO", "-imagekey", "zlib-level=9", "-ov", "-o"]).arg(dmg).arg(&writable))?;
    }
    for done in [writable, one, two] {
        let _ = std::fs::remove_file(done);
    }
    Ok(())
}

/// Finder's instructions for the window: no toolbar, big icons, the picture,
/// and each icon where the picture expects it.
fn layout_script(mount: &Path) -> String {
    use background::{APPLICATIONS_AT, APP_AT, HEIGHT, ICON_SIZE, READ_ME_AT, WIDTH};
    // The window's frame includes its title bar.
    let (left, top) = (200, 120);
    format!(
        r#"with timeout of 120 seconds
tell application "Finder"
  tell (item (POSIX file "{mount}" as alias))
    open
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    set the bounds of container window to {{{left}, {top}, {right}, {bottom}}}
    set options to the icon view options of container window
    set arrangement of options to not arranged
    set icon size of options to {ICON_SIZE}
    set text size of options to 13
    set background picture of options to (POSIX file "{mount}/.background/background.tiff" as alias)
    set position of item "{NAME}.app" of container window to {{{ax}, {ay}}}
    set position of item "Applications" of container window to {{{bx}, {by}}}
    set position of item "Read Me First.txt" of container window to {{{cx}, {cy}}}
    update without registering applications
    delay 1
    close
    open
    delay 1
    close
  end tell
end tell
end timeout"#,
        mount = mount.display(),
        right = left + WIDTH,
        bottom = top + HEIGHT + 28,
        ax = APP_AT.0,
        ay = APP_AT.1,
        bx = APPLICATIONS_AT.0,
        by = APPLICATIONS_AT.1,
        cx = READ_ME_AT.0,
        cy = READ_ME_AT.1,
    )
}

/// The version shown: the package's, with the commit it was built from.
fn version(root: &Path) -> Result<String, String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|error| error.to_string())?;
    let package = manifest
        .lines()
        .find_map(|line| line.strip_prefix("version = ").map(|v| v.trim_matches('"').to_owned()))
        .ok_or("no version in Cargo.toml")?;
    let commit = output(Command::new("git").arg("-C").arg(root).args(["rev-parse", "--short", "HEAD"]));
    let dirty = output(Command::new("git").arg("-C").arg(root).args(["status", "--porcelain", "--untracked-files=no"])).is_some_and(|text| !text.is_empty());
    Ok(match commit {
        Some(commit) => format!("{package}-{commit}{}", if dirty { "-modified" } else { "" }),
        None => package,
    })
}

/// Builds the game for players, returning the program.
fn build(root: &Path, mac: Mac, no_build: bool) -> Result<PathBuf, String> {
    let installed = output(Command::new("rustup").args(["target", "list", "--installed"])).unwrap_or_default();
    if !installed.lines().any(|line| line == mac.target) {
        return Err(format!("the {} target isn't installed: run `rustup target add {}`", mac.target, mac.target));
    }
    if !no_build {
        run(Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
            .current_dir(root)
            .args(["build", "--profile", "dist", "--bin", EXECUTABLE, "--target", mac.target])
            .env("MACOSX_DEPLOYMENT_TARGET", mac.minimum))?;
    }
    let program = root.join("target").join(mac.target).join("dist").join(EXECUTABLE);
    program.exists().then_some(program.clone()).ok_or(format!("{} wasn't built", program.display()))
}

/// The app's description of itself.
fn info_plist(id: &str, version: &str, minimum: &str) -> String {
    // Why the app reads files outside its own folder, shown by macOS when it
    // asks the player's leave to read the game's folder there.
    let why = "RA2 Engine reads the game's files from the folder you chose for them.";
    let short = version.split('-').next().unwrap_or(version);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key><string>en</string>
	<key>CFBundleDisplayName</key><string>{NAME}</string>
	<key>CFBundleName</key><string>{NAME}</string>
	<key>CFBundleExecutable</key><string>{EXECUTABLE}</string>
	<key>CFBundleIdentifier</key><string>{id}</string>
	<key>CFBundleIconFile</key><string>AppIcon</string>
	<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>{short}</string>
	<key>CFBundleVersion</key><string>{short}</string>
	<key>RA2BuildVersion</key><string>{version}</string>
	<key>LSMinimumSystemVersion</key><string>{minimum}</string>
	<key>LSApplicationCategoryType</key><string>public.app-category.strategy-games</string>
	<key>NSHighResolutionCapable</key><true/>
	<key>NSSupportsAutomaticGraphicsSwitching</key><true/>
	<key>NSHumanReadableCopyright</key><string>A rewrite of the game's engine. It holds none of Red Alert 2's files and plays from the player's own copy.</string>
	<key>NSDocumentsFolderUsageDescription</key><string>{why}</string>
	<key>NSDesktopFolderUsageDescription</key><string>{why}</string>
	<key>NSDownloadsFolderUsageDescription</key><string>{why}</string>
	<key>NSRemovableVolumesUsageDescription</key><string>{why}</string>
	<key>NSNetworkVolumesUsageDescription</key><string>{why}</string>
</dict>
</plist>
"#
    )
}

/// Draws the icon at each size macOS wants and puts them together.
fn make_icon(out: &Path, icns: &Path) -> Result<(), String> {
    let set = out.join("AppIcon.iconset");
    std::fs::create_dir_all(&set).map_err(|error| error.to_string())?;
    for size in [16u32, 32, 128, 256, 512] {
        for scale in [1u32, 2] {
            let name = if scale == 1 { format!("icon_{size}x{size}.png") } else { format!("icon_{size}x{size}@2x.png") };
            icon::draw(size * scale).save(set.join(name)).map_err(|error| error.to_string())?;
        }
    }
    run(Command::new("iconutil").args(["--convert", "icns", "--output"]).arg(icns).arg(&set))?;
    std::fs::remove_dir_all(&set).map_err(|error| error.to_string())
}

/// Fails if the app holds anything but what is put there here: the game's
/// own files must never be handed out with it.
fn holds_only_ours(app: &Path) -> Result<(), String> {
    let ours = ["Contents/Info.plist", "Contents/PkgInfo", &format!("Contents/MacOS/{EXECUTABLE}"), "Contents/Resources/AppIcon.icns"];
    let mut found = Vec::new();
    let mut stack = vec![app.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|error| error.to_string())?.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(path.strip_prefix(app).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    let strangers: Vec<&String> = found.iter().filter(|path| !ours.contains(&path.as_str())).collect();
    if strangers.is_empty() { Ok(()) } else { Err(format!("the app holds files it shouldn't: {strangers:?}")) }
}

/// Signs the app: with a Developer ID (and the hardened runtime notarizing
/// asks for) if one is given, otherwise ad hoc.
fn sign(app: &Path, identity: Option<&str>) -> Result<(), String> {
    let mut codesign = Command::new("codesign");
    codesign.args(["--force", "--deep"]);
    match identity {
        Some(identity) => codesign.args(["--options", "runtime", "--timestamp", "--sign", identity]),
        None => codesign.args(["--sign", "-"]),
    };
    run(codesign.arg(app))?;
    run(Command::new("codesign").args(["--verify", "--deep", "--strict"]).arg(app))
}

fn read_me(version: &str, notarized: bool) -> String {
    let opening = if notarized {
        String::new()
    } else {
        format!(
            "OPENING IT THE FIRST TIME
This build isn't notarized by Apple, so the first time you open it macOS
says it can't check it. To open it:
  - macOS 15 (Sequoia) and later: open it once, choose Done, then go to
    System Settings > Privacy & Security, scroll down and choose
    \"Open Anyway\" beside \"{NAME}\".
  - Earlier macOS: Control-click (or right-click) the app in
    Applications, choose Open, then Open again.
You only need to do this once.

"
        )
    };
    format!(
        "{NAME}: an engine that plays Red Alert 2 from your own copy of the game, version {version}

INSTALLING
Drag \"{NAME}\" onto the Applications folder beside it.

YOU NEED YOUR OWN COPY OF RED ALERT 2
This app holds none of the game's files. The first time it runs it asks
for the folder your copy of Red Alert 2 is installed in (from Steam or
the EA app, or installed from the CDs) and remembers it. If that folder
goes away (an external drive unplugged, the game moved or uninstalled)
it asks again; your saved games and settings are kept either way.

Own it on Steam? Steam for Mac won't install it (the Install button does
nothing), but the setup window's \"Download with Steam\" fetches the files
through Steam's console for you.

To choose a different folder later, hold the Option key while the game
starts.

{opening}WHERE YOUR THINGS ARE KEPT
Saved games and settings: ~/Library/Application Support/RA2 Engine
The log (useful when reporting a problem): ~/Library/Logs/RA2 Engine/ra2.log
"
    )
}
