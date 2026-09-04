//! `device.*` — BUS.md §10.14. `avd.*` is reserved for post-4.0.
use crate::registry::{Actors, Audit, OpMeta, Scope};
use crate::types::{Avd, Device, Id, Run};
use crate::{op, Empty};
use serde_json::Value;

result!(#[schemars(rename = "DeviceListOut")] ListOut { pub devices: Vec<Device> });
op!(List, "device.list", Empty => ListOut, OpMeta::query(Scope::Global, 10, "adb devices (and AVDs later)"));
payload!(#[schemars(rename = "DeviceWatchIn")] WatchIn { pub on: bool });
op!(Watch, "device.watch", WatchIn => Empty,
    OpMeta::mutation(Scope::Global, 10, "While device control is visible, emit device.changed from adb track-devices").audit(Audit::Never).actors(Actors::UserOnly).emits(&["device.changed"]));
payload!(#[schemars(rename = "DeviceMirrorStartIn")] MirrorStartIn { pub device: String, pub max_size: Option<u32>, pub bitrate: Option<u32> });
result!(#[schemars(rename = "DeviceMirrorStartOut")] MirrorStartOut { pub mirror_id: Id, pub width: u32, pub height: u32 });
op!(MirrorStart, "device.mirror.start", MirrorStartIn => MirrorStartOut,
    OpMeta::mutation(Scope::Global, 10, "Start a scrcpy-class H.264 mirror; frames on a dedicated native or Tauri connection").actors(Actors::UserOnly).stream("mirror").emits(&["mirror.changed"]));
payload!(#[schemars(rename = "DeviceMirrorIdIn")] MirrorIdIn { pub mirror_id: Id });
op!(MirrorStop, "device.mirror.stop", MirrorIdIn => Empty, OpMeta::mutation(Scope::Global, 10, "Stop a mirror").actors(Actors::UserOnly).emits(&["mirror.changed"]));
payload!(#[schemars(rename = "DeviceMirrorInputIn")] MirrorInputIn { pub mirror_id: Id, pub event: Value });
op!(MirrorInput, "device.mirror.input", MirrorInputIn => Empty,
    OpMeta::mutation(Scope::Global, 10, "Touch / key event to the device").audit(Audit::Never).actors(Actors::UserOnly));
payload!(#[schemars(rename = "DeviceRunIn")] RunIn { pub project_id: Id, pub worktree: Option<String>, pub device: String, pub variant: Option<String>, pub integration_id: Option<Id> });
op!(RunOp, "device.run", RunIn => Run,
    OpMeta::mutation(Scope::Project, 10, "gradle installDebug then stream logcat; crashes as run.crash events").actors(Actors::UserOnly).stream("logcat").emits(&["run.changed", "run.crash", "notify.new"]));
payload!(#[schemars(rename = "DeviceBuildIn")] BuildIn { pub project_id: Id, pub worktree: Option<String>, pub variant: Option<String>, pub format: Option<String>, pub publish: Option<bool>, pub integration_id: Option<Id> });
op!(Build, "device.build", BuildIn => Run,
    OpMeta::mutation(Scope::Project, 10, "gradle assemble/bundle <Variant> with no device attached; record artifact signing; publish uploads to Google Play").actors(Actors::UserOnly).stream("logcat").emits(&["run.changed"]));
payload!(#[schemars(rename = "DeviceSigningGetIn")] SigningGetIn { pub project_id: Id });
result!(#[schemars(rename = "DeviceSigningProfileOut")] SigningProfileOut {
    pub configured: bool,
    pub enabled: bool,
    pub key_alias: Option<String>,
    pub keystore: Option<String>,
});
op!(SigningGet, "device.signing.get", SigningGetIn => SigningProfileOut,
    OpMeta::query(Scope::Project, 10, "Relay-owned Android release signing profile metadata, never its password").actors(Actors::UserOnly));
payload!(#[schemars(rename = "DeviceSigningCreateIn")] SigningCreateIn { pub project_id: Id, pub key_alias: String, pub password: String });
op!(SigningCreate, "device.signing.create", SigningCreateIn => SigningProfileOut,
    OpMeta::mutation(Scope::Project, 10, "Create a Relay-owned Android release key and save its password in Linux Secret Service")
        .audit(Audit::Never).actors(Actors::UserOnly).emits(&["device.signing.changed"]));
payload!(#[schemars(rename = "DeviceSigningSetEnabledIn")] SigningSetEnabledIn { pub project_id: Id, pub enabled: bool });
op!(SigningSetEnabled, "device.signing.set_enabled", SigningSetEnabledIn => SigningProfileOut,
    OpMeta::mutation(Scope::Project, 10, "Choose Relay-owned or project-owned Android release signing")
        .actors(Actors::UserOnly).emits(&["device.signing.changed"]));
payload!(#[schemars(rename = "DeviceRunIdIn")] RunIdIn { pub run_id: Id });
op!(RunStop, "device.run.stop", RunIdIn => Empty, OpMeta::mutation(Scope::Global, 10, "Stop a run").actors(Actors::UserOnly).emits(&["run.changed"]));
payload!(#[schemars(rename = "DeviceRunListIn")] RunListIn { pub project_id: Id });
result!(#[schemars(rename = "DeviceRunListOut")] RunListOut { pub runs: Vec<Run> });
op!(RunList, "device.run.list", RunListIn => RunListOut, OpMeta::query(Scope::Project, 10, "Runs of a project"));

result!(#[schemars(rename = "AvdListOut")] AvdListOut { pub avds: Vec<Avd> });
op!(AvdList, "avd.list", Empty => AvdListOut, OpMeta::query(Scope::Global, 12, "Installed Android Virtual Devices and running serials"));
result!(#[schemars(rename = "AvdCatalogOut")] AvdCatalogOut { pub system_images: Vec<String>, pub devices: Vec<String> });
op!(AvdCatalog, "avd.catalog", Empty => AvdCatalogOut, OpMeta::query(Scope::Global, 12, "Installed system images and device profiles available for AVD creation"));
payload!(#[schemars(rename = "AvdCreateIn")] AvdCreateIn { pub name: String, pub package: String, pub device: Option<String> });
op!(AvdCreate, "avd.create", AvdCreateIn => Avd,
    OpMeta::mutation(Scope::Global, 12, "Create an Android Virtual Device").actors(Actors::UserOnly).emits(&["avd.changed"]));
payload!(#[schemars(rename = "AvdBootIn")] AvdBootIn { pub name: String, pub cold: Option<bool> });
op!(AvdBoot, "avd.boot", AvdBootIn => Empty,
    OpMeta::mutation(Scope::Global, 12, "Boot an Android Virtual Device").actors(Actors::UserOnly).emits(&["avd.changed"]));

entries!(
    List,
    Watch,
    MirrorStart,
    MirrorStop,
    MirrorInput,
    RunOp,
    Build,
    SigningGet,
    SigningCreate,
    SigningSetEnabled,
    RunStop,
    RunList,
    AvdList,
    AvdCatalog,
    AvdCreate,
    AvdBoot
);
