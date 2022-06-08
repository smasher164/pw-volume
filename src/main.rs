use anyhow::{anyhow, ensure};
use clap::{App, AppSettings, Arg, ArgMatches, SubCommand};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::process::Command;

#[derive(Deserialize, Debug, PartialEq)]
#[serde(untagged)]
enum PipeWireObject<'a> {
    #[serde(borrow)]
    Metadata(PipeWireInterfaceMetadata<'a>),

    #[serde(borrow)]
    Node(PipeWireInterfaceNode<'a>),

    #[serde(borrow)]
    Device(PipeWireInterfaceDevice<'a>),
    Value(Value),
}

#[derive(Deserialize, Debug, PartialEq)]
struct PipeWireInterfaceDevice<'a> {
    id: i64,

    #[serde(rename = "type")]
    typ: &'a str,

    #[serde(borrow)]
    info: DeviceInfo<'a>,
}

#[derive(Deserialize, Debug, PartialEq)]
struct DeviceInfo<'a> {
    #[serde(borrow)]
    params: DeviceParams<'a>,
}

#[derive(Deserialize, Debug, PartialEq)]
struct DeviceParams<'a> {
    #[serde(borrow)]
    #[serde(rename = "Route")]
    route: Vec<DeviceRoute<'a>>,
}

#[derive(Deserialize, Debug, PartialEq)]
struct DeviceRoute<'a> {
    index: i64,
    direction: &'a str,
    props: DeviceRouteProp,
}

#[derive(Deserialize, Debug, PartialEq)]
struct DeviceRouteProp {
    mute: bool,
    #[serde(rename = "volumeBase")]
    volume_base: f64,
    #[serde(rename = "channelVolumes")]
    channel_volumes: Vec<f64>,
}

#[derive(Deserialize, Debug, PartialEq)]
struct PipeWireInterfaceNode<'a> {
    id: i64,

    #[serde(rename = "type")]
    typ: &'a str,

    #[serde(borrow)]
    info: NodeInfo<'a>,
}

#[derive(Deserialize, Debug, PartialEq)]
struct NodeInfo<'a> {
    #[serde(borrow)]
    props: NodeProps<'a>,

    #[serde(borrow)]
    params: NodeParams<'a>,
}

#[derive(Deserialize, Debug, PartialEq)]
struct NodeProps<'a> {
    #[serde(rename = "card.profile.device")]
    card_profile_device: i64,

    #[serde(rename = "device.id")]
    device_id: i64,

    #[serde(rename = "node.name")]
    node_name: &'a str,
}

#[derive(Deserialize, Debug, PartialEq)]
struct NodeParams<'a> {
    #[serde(rename = "EnumFormat")]
    enum_format: Vec<NodeEnumFormat>,

    #[serde(borrow)]
    #[serde(rename = "PropInfo")]
    prop_info: Vec<NodePropInfo<'a>>,

    #[serde(rename = "Props")]
    props: Vec<NodeProp>,
}

#[derive(Deserialize, Debug, PartialEq)]
struct NodeEnumFormat {
    channels: Option<i64>,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(untagged)]
enum NodePropInfo<'a> {
    #[serde(borrow)]
    Volume(NodePropInfoVolume<'a>),
    Value(Value),
}

#[derive(Deserialize, Debug, PartialEq)]
struct NodePropInfoVolume<'a> {
    id: &'a str,

    #[serde(rename = "type")]
    typ: NodePropInfoTypeVolume,
}

#[derive(Deserialize, Debug, PartialEq)]
struct NodePropInfoTypeVolume {
    default: f64,
    min: f64,
    max: f64,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(untagged)]
enum NodeProp {
    Volume(NodePropVolume),
    Value(Value),
}

#[derive(Deserialize, Debug, PartialEq)]
struct NodePropVolume {
    volume: f64,
    mute: bool,

    #[serde(rename = "channelVolumes")]
    channel_volumes: Vec<f64>,
}

#[derive(Deserialize, Debug, PartialEq)]
struct PipeWireInterfaceMetadata<'a> {
    #[serde(rename = "type")]
    typ: &'a str,

    #[serde(borrow)]
    metadata: Vec<Metadata<'a>>,
}

#[derive(Deserialize, Debug, PartialEq)]
struct Metadata<'a> {
    key: &'a str,

    #[serde(borrow)]
    value: MetadataValue<'a>,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(untagged)]
enum MetadataValue<'a> {
    #[serde(borrow)]
    Name(MetadataValueName<'a>),
    Value(Value),
}

#[derive(Deserialize, Debug, PartialEq)]
struct MetadataValueName<'a> {
    name: &'a str,
}

#[derive(Serialize, Debug, Default)]
struct PipeWireCommand {
    index: i64,
    device: i64,
    props: CommandVolumeProps,
}

#[derive(Serialize, Debug, Default)]
struct CommandVolumeProps {
    mute: bool,

    #[serde(rename = "channelVolumes")]
    channel_volumes: Vec<f64>,
}

fn is_decimal_percentage(value: &str) -> bool {
    value
        .strip_suffix('%')
        .and_then(|value| value.parse::<f32>().ok())
        .is_some()
}

fn parse_dump<'a>(
    obj: &'a [PipeWireObject<'_>],
) -> anyhow::Result<(&'a PipeWireInterfaceNode<'a>, &'a DeviceRoute<'a>)> {
    // find the default audio sink from the dump
    let default_audio_sink = obj
        .iter()
        .filter_map(|o| match o {
            PipeWireObject::Metadata(md) if md.typ == "PipeWire:Interface:Metadata" => Some(md),
            _ => None,
        })
        .flat_map(|md| &md.metadata)
        .find_map(|md| match &md.value {
            MetadataValue::Name(mv) if md.key == "default.audio.sink" => Some(mv.name),
            _ => None,
        })
        .ok_or_else(|| anyhow!("failed to determine default audio sink"))?;

    // find node whose default audio sink is ours
    let node = obj
        .iter()
        .find_map(|o| match o {
            PipeWireObject::Node(n)
                if n.typ == "PipeWire:Interface:Node"
                    && n.info.props.node_name == default_audio_sink =>
            {
                Some(n)
            }
            _ => None,
        })
        .ok_or_else(|| anyhow!("failed to find node for audio sink: {}", default_audio_sink))?;

    // get device corresponding to this node
    let device = obj
        .iter()
        .find_map(|o| match o {
            PipeWireObject::Device(d)
                if d.typ == "PipeWire:Interface:Device" && d.id == node.info.props.device_id =>
            {
                Some(d)
            }
            _ => None,
        })
        .ok_or_else(|| anyhow!("failed to find device: {}", node.info.props.device_id))?;

    // get active route for audio output
    let route = device
        .info
        .params
        .route
        .iter()
        .find(|r| r.direction == "Output")
        .ok_or_else(|| anyhow!("failed to find output route"))?;

    ensure!(
        !route.props.channel_volumes.is_empty(),
        "no volume channels present"
    );
    Ok((node, route))
}

fn pw_cli<'a>(
    matches: &ArgMatches<'_>,
    node: &'a PipeWireInterfaceNode<'a>,
    route: &'a DeviceRoute<'a>,
) -> anyhow::Result<()> {
    // build and send a command to pw-cli to update audio state
    let mut cmd = PipeWireCommand {
        index: route.index,
        device: node.info.props.card_profile_device,
        ..Default::default()
    };
    match matches.subcommand() {
        ("mute", Some(arg)) => match arg.value_of("TRANSITION") {
            Some("on") => cmd.props.mute = true,
            Some("toggle") => cmd.props.mute = !route.props.mute,
            _ => (), // Some("off") => cmd.mute is already false
        },
        ("change", Some(arg)) => {
            let delta = arg
                .value_of("DELTA")
                .ok_or_else(|| anyhow!("DELTA argument not found"))?;
            let percent = &delta[..delta.len() - 1].parse::<f64>()?;
            let increment = percent * 0.01;
            let mut vols = Vec::with_capacity(route.props.channel_volumes.len());
            for vol in route.props.channel_volumes.iter() {
                let new_vol = (vol + increment).clamp(0.0, 1.0);
                vols.push(new_vol);
            }
            cmd.props.channel_volumes = vols;
        }
        ("status", _) => {
            if route.props.mute {
                println!(r#"{{"alt":"mute", "tooltip":"muted"}}"#);
            } else {
                // assumes that all channels have the same volume.
                let vol = route.props.channel_volumes[0];
                let percentage = vol * 100.0;
                println!(
                    r#"{{"percentage":{:.0}, "tooltip":"{}%"}}"#,
                    percentage, percentage
                );
            }
            return Ok(());
        }
        (_, _) => unreachable!("argument parsing should have failed by now"),
    };
    let set_cmd = serde_json::to_string(&cmd)?;
    let code = Command::new("pw-cli")
        .args([
            "set-param",
            &node.info.props.device_id.to_string(),
            "Route",
            &set_cmd,
        ])
        .spawn()?
        .wait()?
        .code()
        .ok_or_else(|| anyhow!("pw-cli terminated by signal"))?;
    ensure!(code == 0, "pw-cli did not exit successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Read, path::PathBuf};
    use test_case::test_case;

    use super::*;

    #[test_case("without_discord.txt")]
    #[test_case("with_discord.txt")]
    fn parse_output(filename: &str) -> anyhow::Result<()> {
        let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src", "testdata", filename]
            .iter()
            .collect();
        let mut f = File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        let obj: Vec<PipeWireObject> = serde_json::from_slice(&buf)?;
        parse_dump(&obj)?;
        Ok(())
    }
}

fn main() {
    // parse cli flags
    let matches = App::new("pw-volume")
        .about("Basic interface to PipeWire volume controls")
        .settings(&[
            AppSettings::SubcommandRequiredElseHelp,
            AppSettings::DisableVersion,
            AppSettings::VersionlessSubcommands,
            AppSettings::UnifiedHelpMessage,
            AppSettings::DisableHelpSubcommand,
        ])
        .subcommand(
            SubCommand::with_name("mute")
                .about("mutes audio [possible values: on, off, toggle]")
                .setting(AppSettings::ArgRequiredElseHelp)
                .arg(
                    Arg::with_name("TRANSITION")
                        .takes_value(true)
                        .required(true)
                        .possible_values(&["on", "off", "toggle"]),
                ),
        )
        .subcommand(
            SubCommand::with_name("change")
                .about("adjusts volume by decimal percentage, e.g. '+1%', '-0.5%'")
                .setting(AppSettings::ArgRequiredElseHelp)
                .setting(AppSettings::AllowLeadingHyphen)
                .arg(
                    Arg::with_name("DELTA")
                        .help("decimal percentage, e.g. '+1%', '-0.5%'")
                        .takes_value(true)
                        .required(true)
                        .allow_hyphen_values(true)
                        .validator(move |s| {
                            if is_decimal_percentage(&s) {
                                Ok(())
                            } else {
                                Err(format!(r#""{}" is not a decimal percentage"#, s))
                            }
                        }),
                ),
        )
        .subcommand(SubCommand::with_name("status").about("get volume and mute information"))
        .get_matches();

    // call pw-dump and unmarshal its output
    let output = Command::new("pw-dump")
        .output()
        .expect("failed to execute pw-dump");
    let obj: Vec<PipeWireObject> =
        serde_json::from_slice(&output.stdout).expect("failed to unmarshal PipeWireObject");
    let (node, route) = parse_dump(&obj).unwrap();
    pw_cli(&matches, node, route).unwrap();
}
