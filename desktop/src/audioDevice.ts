import type { AudioDevice } from "./ipc";

/**
 * Display label for an audio device in the Settings pickers.
 *
 * ALSA exposes generic C-Media USB codecs (the DE-19 among them, but also many
 * other CM108-style interfaces) with the card name "Device". Keep the real
 * identifier visible while making it recognizable, and steer toward the
 * `plughw:` variant, which resamples to the rate we ask for.
 *
 * The QMX's own USB Audio Class descriptor registers its ALSA card ID as
 * "Transceiver" (confirmed via `cat /proc/asound/cards`: `[Transceiver]:
 * USB-Audio - QMX Transceiver`) -- a raw `plughw:CARD=Transceiver,DEV=0`/
 * `sysdefault:CARD=Transceiver` name carries no recognizable "QMX" text at
 * all, even though cpal enumerates it correctly (confirmed live: it's
 * genuinely present in both the input and output device lists). This was
 * reported as "the QMX doesn't appear in Audio Input/Output" -- it DOES
 * appear, just unrecognizably, unlike PulseAudio's own friendlier
 * "QRP Labs QMX Transceiver" description shown in pavucontrol. Same fix
 * shape as the C-Media case above: keep the real identifier, prefix a
 * recognizable name.
 */
export function audioDeviceLabel(device: AudioDevice): string {
  if (device.name.includes("CARD=Device")) {
    const recommended = device.name.startsWith("plughw:") ? " — recommended" : "";
    return `USB audio codec (C-Media, e.g. DE-19) — ${device.name}${recommended}`;
  }
  if (device.name.includes("CARD=Transceiver")) {
    const recommended = device.name.startsWith("plughw:") ? " — recommended" : "";
    return `QRP Labs QMX Transceiver — ${device.name}${recommended}`;
  }
  if (device.name === "pipewire") return "PipeWire sound server";
  if (device.name === "default") return "Linux system default";
  return device.name;
}
