# pw-volume

```
Basic interface to PipeWire volume controls

USAGE:
    pw-volume <SUBCOMMAND>

OPTIONS:
    -h, --help    Prints help information

SUBCOMMANDS:
    change    adjusts volume by decimal percentage, e.g. '+1%', '-0.5%'
    mute      mutes audio [possible values: on, off, toggle]
    status    get volume and mute information
```

### Example Usage

You can use pw-volume to bind multimedia keys to raise, lower, and mute volume.
For instance, in Sway's config:

```
bindsym XF86AudioRaiseVolume exec pw-volume change +2.5%
bindsym XF86AudioLowerVolume exec pw-volume change -2.5%
bindsym XF86AudioMute exec pw-volume mute toggle
```