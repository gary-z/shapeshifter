"""Build the measurement driver against a chosen checkout and its locked dependencies."""
import argparse
import json
import shutil
import subprocess
import tempfile
import tomllib
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('checkout', type=Path)
parser.add_argument('binary', type=Path)
args = parser.parse_args()
checkout = args.checkout.resolve()
binary = args.binary.resolve()
with tempfile.TemporaryDirectory(prefix='shapeshifter-measure-') as temporary:
    driver = Path(temporary)
    (driver / 'src').mkdir()
    shutil.copyfile(Path(__file__).with_name('measure.rs'), driver / 'src/main.rs')
    shutil.copyfile(checkout / 'Cargo.lock', driver / 'Cargo.lock')
    # JSON string quoting is also valid for these TOML path strings.
    (driver / 'Cargo.toml').write_text('''[package]
name = "native-measure"
version = "0.1.0"
edition = "2024"

[dependencies]
shapeshifter = { path = ''' + json.dumps(str(checkout), ensure_ascii=False) + ''' }
serde_json = "1.0.149"

[profile.release]
lto = true
codegen-units = 1
panic = "abort"
strip = true
''')
    command = ['cargo', 'build', '--release', '--offline', '--manifest-path', str(driver / 'Cargo.toml'),
               '--target-dir', str(driver / 'target')]
    # Cargo discovers the checkout's pinned compiler and native CPU flags from cwd.
    # Only the temporary root package is new; dependency versions come from its lockfile copy.
    subprocess.run(command, cwd=checkout, check=True)
    def dependencies(path):
        packages = tomllib.loads(path.read_text())['package']
        return {(p['name'], p['version'], p.get('source')) for p in packages
                if p['name'] != 'native-measure'}

    # Unused optional browser dependencies can disappear from the driver lockfile.
    assert dependencies(driver / 'Cargo.lock') <= dependencies(checkout / 'Cargo.lock'), \
        'Measurement dependencies differ from the checkout lockfile'
    binary.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(driver / 'target/release/native-measure', binary)
