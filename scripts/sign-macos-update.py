#!/usr/bin/env python3
"""Create a signed per-architecture Sparkle feed after the final DMG has been built."""
import argparse
import base64
import os
import pathlib
import plistlib
import subprocess
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent
SPARKLE = 'http://www.andymatuschak.org/xml-namespaces/sparkle'
ET.register_namespace('sparkle', SPARKLE)


def create_feed(directory, version, arch, build, sign, key, public_key, notes):
    name = f'Aieyes-{version}-macos-{arch}.dmg'
    package = directory / name
    # sign_update accepts the key through stdin; it never appears in argv or logs.
    signature = subprocess.run([str(sign), '--ed-key-file', '-', '-p', str(package)], input=key, text=True, capture_output=True, check=True).stdout.strip()
    subprocess.run([str(sign), '--ed-key-file', '-', '--verify', str(package), signature], input=key, text=True, capture_output=True, check=True)
    if len(base64.b64decode(signature, validate=True)) != 64:
        raise ValueError('Invalid Sparkle package signature')
    rss = ET.Element('rss', version='2.0')
    channel = ET.SubElement(rss, 'channel')
    ET.SubElement(channel, 'title').text = 'Aieyes'
    item = ET.SubElement(channel, 'item')
    ET.SubElement(item, 'title').text = f'Aieyes {version}'
    ET.SubElement(item, 'description').text = notes
    ET.SubElement(item, f'{{{SPARKLE}}}version').text = build
    ET.SubElement(item, f'{{{SPARKLE}}}shortVersionString').text = version
    ET.SubElement(item, f'{{{SPARKLE}}}minimumSystemVersion').text = '14.0'
    if arch == 'arm64':
        ET.SubElement(item, f'{{{SPARKLE}}}hardwareRequirements').text = 'arm64'
    ET.SubElement(item, 'enclosure', url=f'https://github.com/JesmonX/Aieyes/releases/download/v{version}/{name}', length=str(package.stat().st_size), type='application/octet-stream', **{f'{{{SPARKLE}}}edSignature': signature})
    feed = directory / f'appcast-macos-{arch}.xml'
    ET.indent(rss)
    ET.ElementTree(rss).write(feed, encoding='utf-8', xml_declaration=True)
    subprocess.run([str(sign), '--ed-key-file', '-', str(feed)], input=key, text=True, capture_output=True, check=True)
    # Verification uses the same key; build also checks public/private correspondence below.
    subprocess.run([str(sign), '--ed-key-file', '-', '--verify', str(feed)], input=key, text=True, capture_output=True, check=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--directory', type=pathlib.Path, default=ROOT/'dist/release')
    parser.add_argument('--arch', choices=['x64','arm64'], required=True)
    args = parser.parse_args()
    try:
        info = plistlib.loads((ROOT/'dist/Aieyes.app/Contents/Info.plist').read_bytes())
        key = os.environ['SPARKLE_PRIVATE_KEY'].strip()
        public_key = os.environ['SPARKLE_PUBLIC_KEY'].strip()
        if info.get('SUPublicEDKey') != public_key:
            raise ValueError('Embedded Sparkle public key does not match release configuration')
        # CryptoKit is available on the macOS builder; check the seed without exposing it.
        check = subprocess.run(['swift', str(ROOT/'scripts/verify-sparkle-key.swift')], input=key+'\n'+public_key+'\n', text=True, capture_output=True)
        if check.returncode:
            raise ValueError('Sparkle private/public key mismatch (use a current 32-byte seed export)')
        notes_path = ROOT/'dist/release-notes.txt'
        notes = notes_path.read_text() if notes_path.exists() else f'Aieyes {info["CFBundleShortVersionString"]}：改进与修复。'
        create_feed(args.directory, info['CFBundleShortVersionString'], args.arch, info['CFBundleVersion'], ROOT/'.build/sparkle-tools/bin/sign_update', key, public_key, notes)
    except (KeyError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'Sparkle signing failed: {type(error).__name__}\n')
