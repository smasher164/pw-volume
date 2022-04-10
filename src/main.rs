use clap::{App, AppSettings, Arg, ArgMatches, SubCommand};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{error, process::Command};

#[derive(Deserialize, Debug, PartialEq)]
#[serde(untagged)]
enum PipeWireObject<'a> {
    #[serde(borrow)]
    Metadata(PipeWireInterfaceMetadata<'a>),

    #[serde(borrow)]
    Node(PipeWireInterfaceNode<'a>),
    Value(Value),
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
struct MetadataValue<'a> {
    name: &'a str,
}

#[derive(Serialize, Debug, Default)]
struct PipeWireCommand {
    mute: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    volume: Option<f64>,

    #[serde(rename = "channelVolumes")]
    #[serde(skip_serializing_if = "Option::is_none")]
    channel_volumes: Option<Vec<f64>>,
}

fn is_decimal_percentage(value: &str) -> bool {
    value
        .strip_suffix("%")
        .and_then(|value| value.parse::<f32>().ok())
        .is_some()
}

fn pw_dump(
    obj: Vec<PipeWireObject<'_>>,
    matches: &ArgMatches<'_>,
) -> Result<(), Box<dyn error::Error>> {
    // find the default audio sink from the dump
    let default_audio_sink = obj
        .iter()
        .filter_map(|o| match o {
            PipeWireObject::Metadata(md) if md.typ == "PipeWire:Interface:Metadata" => Some(md),
            _ => None,
        })
        .flat_map(|md| &md.metadata)
        .find_map(|md| {
            if md.key == "default.audio.sink" {
                Some(md.value.name)
            } else {
                None
            }
        })
        .ok_or("failed to determine default audio sink")?;

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
        .ok_or(format!(
            "failed to find node for audio sink: {}",
            default_audio_sink
        ))?;

    // read volume property info
    let volume_prop = node
        .info
        .params
        .prop_info
        .iter()
        .find_map(|p| match p {
            NodePropInfo::Volume(v) if v.id == "volume" => Some(&v.typ),
            _ => None,
        })
        .ok_or(format!(
            "failed to determine volume range for node: {}",
            node.id
        ))?;

    // like min and max to compute the range
    let range = volume_prop.max - volume_prop.min;
    // in case JSON from volume range is invalid
    if range <= 0.0 {
        return Err(Box::from(format!(
            "volume range ({}, {}) is not positive",
            volume_prop.min, volume_prop.max,
        )));
    }

    // read the current volume and mute status
    let status = node
        .info
        .params
        .props
        .iter()
        .find_map(|p| match p {
            NodeProp::Volume(v) => Some(v),
            _ => None,
        })
        .ok_or(format!("failed to determine volume for node: {}", node.id))?;

    // build and send a command to pw-cli to update audio state
    let mut cmd: PipeWireCommand = Default::default();
    match matches.subcommand() {
        ("mute", Some(arg)) => match arg.value_of("TRANSITION") {
            Some("on") => cmd.mute = true,
            Some("toggle") => cmd.mute = !status.mute,
            _ => (), // Some("off") => cmd.mute is already false
        },
        ("change", Some(arg)) => {
            let delta = arg.value_of("DELTA").ok_or("DELTA argument not found")?;
            let percent = &delta[..delta.len() - 1].parse::<f64>()?;
            let increment = percent * range / 100.0;
            let new_vol = (status.volume + increment).clamp(volume_prop.min, volume_prop.max);
            cmd.volume = Some(new_vol);
        }
        ("status", _) => {
            if status.mute {
                println!(r#"{{"alt":"mute", "tooltip":"muted"}}"#);
            } else {
                let percentage = (status.volume * 100.0) / range;
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
        .args(["set-param", &node.id.to_string(), "Props", &set_cmd])
        .spawn()?
        .wait()?
        .code()
        .ok_or("pw-cli terminated by signal")?;
    if code != 0 {
        return Err(Box::from("pw-cli did not exit successfully"));
    }
    Ok(())
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

    pw_dump(obj, &matches).unwrap();
}
