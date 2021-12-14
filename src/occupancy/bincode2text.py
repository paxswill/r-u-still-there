#!/usr/bin/env python3
"""Helper for converting r-u-still-there sample recordings into text-based files.

The generated files are formatted so that temperatures from the thermal image
are laid out the same way it would be rendered, making it easier to highlight at
which frame certain events happen.

Typical usage is `./bincode2text.py ./*.bin`. That command will create a series
of files with the same name as the .bin files. If the `-d` (or `--delta`)
flag is given, the temperature values in the YAML files are show as the
difference from the previous frame's value.

The non-delta output is in a format that can be deserialized with serde to
RecordedData. Unfortunately the TOML output cannot be used for this as there
isn't a way to represent a root-level array in TOML.
"""

import argparse
import contextlib
from dataclasses import dataclass
import enum
import mmap
import operator
import pathlib
import struct
import sys
import textwrap
from typing import List

@dataclass
class MeasurementData:
    values: List[List[float]]
    temperature: float
    delay_nanoseconds: int

    """Return the number of bytes this structure takes up in bincode."""
    def struct_size(self):
        return sum((
            BINCODE_BEFORE_SIZE,
            BINCODE_AFTER_SIZE,
            len(self.values) * len(self.values[0]) * BINCODE_FLOAT_SIZE
        ))

# width (U32), height (U32), num_values (U64)
BINCODE_BEFORE_FORMAT = "<2IQ"
BINCODE_BEFORE_SIZE = struct.calcsize(BINCODE_BEFORE_FORMAT)
# Temperature enum tag (U32; 0 is celsius, 1 is fahrenheit), ambient_temperature
# (F32), seconds delay (U64), nanoseconds delay (U32)
BINCODE_AFTER_FORMAT = "<IfQI"
BINCODE_AFTER_SIZE = struct.calcsize(BINCODE_AFTER_FORMAT)
BINCODE_FLOAT_SIZE = struct.calcsize("f")

NANOSECONDS_IN_SECOND = 1_000_000_000

def parse_measurement(buffer, offset=0):
    width, height, num_values = struct.unpack_from(BINCODE_BEFORE_FORMAT, buffer, offset=offset)
    assert (width * height) == num_values
    offset += BINCODE_BEFORE_SIZE
    # each pixel value as an F32, num_values times
    values = list(struct.unpack_from(f"<{num_values}f", buffer, offset=offset))
    offset += num_values * BINCODE_FLOAT_SIZE
    by_row = [ values[i:i + width] for i in range(0, num_values, width) ]
    _, temperature, seconds, subsecond_nanos = struct.unpack_from(BINCODE_AFTER_FORMAT, buffer, offset=offset)
    # Skip updating the offset
    delay_nanoseconds = subsecond_nanos + NANOSECONDS_IN_SECOND * seconds
    assert isinstance(delay_nanoseconds, int)
    return MeasurementData(by_row, temperature, delay_nanoseconds)

def parse_measurements(buffer):
    first_measurement = parse_measurement(buffer)
    measurement_size = first_measurement.struct_size()
    offset = measurement_size
    yield first_measurement
    while (offset + measurement_size) < len(buffer):
        yield parse_measurement(buffer, offset=offset)
        offset += measurement_size

class OutputFormat(enum.Enum):
    YAML = "yml"
    TOML = "toml"

    def extension(self) -> str:
        return f".{self.value}"

    # Manually creating the entries so that the temperatures are aligned.
    def dump(self, output, measurements, as_delta=False):
        elapsed_nanos = 0
        previous_temperature = 0
        previous_image = None
        if as_delta:
            temperature_format = "+010.7f"
        else:
            temperature_format = "010.7f"
        for frame_number, measurement in enumerate(measurements):
            elapsed_nanos += measurement.delay_nanoseconds
            elapsed_seconds = elapsed_nanos // NANOSECONDS_IN_SECOND
            elapsed_minutes = elapsed_seconds // 60
            elapsed_seconds = (
                (elapsed_seconds % 60) +
                (elapsed_nanos % NANOSECONDS_IN_SECOND) / NANOSECONDS_IN_SECOND
            )
            if as_delta:
                temperature_delta = \
                    measurement.temperature - previous_temperature
                previous_temperature = measurement.temperature
                if previous_image is None:
                    image_deltas = measurement.values
                else:
                    paired_rows = (
                        zip(*rows)
                        for rows in zip(measurement.values, previous_image)
                        )
                    image_deltas = [
                        [operator.sub(*values) for values in rows]
                        for rows in paired_rows
                    ]
                previous_image = measurement.values

                temperature = temperature_delta
                image = image_deltas
            else:
                temperature = measurement.temperature
                image = measurement.values
            # Format the values first
            elapsed = f"{elapsed_minutes:02d}:{elapsed_seconds:010.7f}"
            temperature = f"{temperature:{temperature_format}}"
            formatted_rows = textwrap.indent(",\n".join(
                ", ".join(f"{value:{temperature_format}}" for value in row)
                for row in image
            ), "  ")
            image_width = len(image[0])
            image_height = len(image)
            formatted_delay = "{{ secs: {secs}, nanos: {nanos} }}".format(
                secs=(
                    measurement.delay_nanoseconds // NANOSECONDS_IN_SECOND
                ),
                nanos=(
                    measurement.delay_nanoseconds % NANOSECONDS_IN_SECOND
                ),
            )
            # Both TOML and YAML have the same format for comments
            output.write(f"# Frame number: {frame_number}\n")
            output.write(f"# Elapsed time: {elapsed}\n")
            if self is self.YAML:
                output.write(f"- width: {image_width}\n")
                output.write(f"  height: {image_height}\n")
                output.write
                output.write("  values: [\n")
                # Indent again for yaml
                output.write(textwrap.indent(formatted_rows, "  "))
                output.write("\n  ]\n")
                output.write(f"  temperature: {temperature}\n")
                output.write(f"  delay: {formatted_delay}\n")
            elif self is self.TOML:
                # Not sure this is valid TOML, but it's accepted by the toml
                # crate for a root-level array
                output.write("[[]]\n")
                output.write(f"width: {image_width}\n")
                output.write(f"height: {image_height}\n")
                output.write("values: [\n")
                output.write(formatted_rows)
                output.write("\n]\n")
                output.write(f"delay: {formatted_delay}\n")
                output.write(f"temperature: {temperature}\n")
            # Add an extra line for readability
            output.write("\n")

def bincode2yaml(in_path: pathlib.Path, out_format: OutputFormat, as_delta=False):
    if as_delta:
        new_stem = f"{bincode_path.stem}-delta"
        out_base = bincode_path.with_stem(new_stem)
    else:
        out_base = bincode_path
    out_path = out_base.with_suffix(out_format.extension())
    with contextlib.ExitStack() as contexts:
        in_file = contexts.enter_context(in_path.open("r+b"))
        mapped_in = contexts.enter_context(
            mmap.mmap(in_file.fileno(), 0, access=mmap.ACCESS_READ)
        )
        out_file = contexts.enter_context(out_path.open("w"))
        measurements = parse_measurements(mapped_in)
        out_format.dump(out_file, measurements, as_delta=as_delta)

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "-d", "--delta",
        action="store_true",
        help="Display values as the change from the previous value for that position",
    )
    parser.add_argument("inputs", nargs="+", type=pathlib.Path)
    format_group = parser.add_mutually_exclusive_group()
    format_group.add_argument(
        "--yaml",
        "-y",
        action="store_const",
        const=OutputFormat.YAML,
        dest="out_format",
        help="Output in YAML (the default)",
    )
    format_group.add_argument(
        "--toml",
        "-t",
        action="store_const",
        const=OutputFormat.TOML,
        dest="out_format",
        help="Output in TOML",
    )
    parser.set_defaults(out_format=OutputFormat.YAML)
    args = parser.parse_args()
    for bincode_path in args.inputs:
        if bincode_path.suffix != ".bin":
            print(f"Non-bincode input given: {bincode_path}")
            sys.exit(2)
        bincode2yaml(bincode_path, args.out_format, as_delta=args.delta)