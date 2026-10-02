#!/bin/sh
set -eu

if [ "$#" -gt 1 ]; then
    printf 'usage: %s [model-directory]\n' "$0" >&2
    exit 2
fi

model_dir=${1:-${AEGIS_MODEL_DIR:-models}}
[ -n "$model_dir" ] || model_dir=models
case "$model_dir" in
    */) model_dir=${model_dir%/} ;;
esac
[ -n "$model_dir" ] || model_dir=/

command -v curl >/dev/null 2>&1 || { printf 'fetch-models: curl is required\n' >&2; exit 1; }
command -v shasum >/dev/null 2>&1 || { printf 'fetch-models: shasum is required\n' >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { printf 'fetch-models: python3 is required\n' >&2; exit 1; }
mkdir -p "$model_dir"

# Keep staging on the destination filesystem so each final rename is atomic.
tmp_dir=$(mktemp -d "${model_dir%/}/.fetch-models.XXXXXX")
cleanup() {
    rm -rf "$tmp_dir"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

# Use shasum for SHA-256; Python also checks file size and MiniFASNet Git blob
# SHA-1 framing before returning the digest used by checksums.sha256.
verify_model() {
    sha_line=$(shasum -a 256 "$1") || return 1
    sha_hex=${sha_line%% *}
    python3 - "$1" "$2" "$3" "$4" "$5" "$sha_hex" <<'PY'
import hashlib
import os
import stat
import sys

path, name, expected_size, expected_sha256, expected_blob, sha_hex = sys.argv[1:]
pins = {
    "face_detection_yunet_2023mar.onnx": (232589, "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4", None),
    "face_recognition_sface_2021dec.onnx": (38696353, "0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79", None),
    "2.7_80x80_MiniFASNetV2.onnx": (1744126, None, "298096938aea527d6477d03a75de446df5cbcf75"),
    "4_0_0_80x80_MiniFASNetV1SE.onnx": (1743294, None, "69f1a09d7c076f72fd29c96a81ee88e0fba70134"),
}
try:
    size_pin, sha_pin, blob_pin = pins[name]
    if (str(size_pin), sha_pin or "-", blob_pin or "-") != (expected_size, expected_sha256, expected_blob):
        raise ValueError("internal pin mismatch")
    if len(sha_hex) != 64 or any(char not in "0123456789abcdef" for char in sha_hex):
        raise ValueError("invalid shasum SHA-256 output")
    if sha_pin is not None and sha_hex != sha_pin:
        raise ValueError("SHA-256 mismatch")
    info = os.lstat(path)
    if not stat.S_ISREG(info.st_mode):
        raise ValueError("not a regular file")
    if info.st_size != size_pin:
        raise ValueError(f"size {info.st_size}, expected {size_pin}")
    if blob_pin is not None:
        blob = hashlib.sha1()
        blob.update(b"blob " + str(info.st_size).encode("ascii") + b"\0")
        actual_size = 0
        with open(path, "rb") as model:
            while True:
                chunk = model.read(1024 * 1024)
                if not chunk:
                    break
                actual_size += len(chunk)
                blob.update(chunk)
        if actual_size != size_pin:
            raise ValueError(f"size {actual_size}, expected {size_pin}")
        if blob.hexdigest() != blob_pin:
            raise ValueError("Git blob SHA-1 mismatch")
    print(sha_hex)
except (OSError, KeyError, ValueError) as error:
    print(f"fetch-models: {name}: {error}", file=sys.stderr)
    sys.exit(1)
PY
}

while IFS='|' read -r name size sha256 git_blob url; do
    [ -n "$name" ] || continue
    destination="$model_dir/$name"
    staged="$tmp_dir/$name"

    if [ -L "$destination" ] || { [ -e "$destination" ] && [ ! -f "$destination" ]; }; then
        printf 'fetch-models: refusing to replace non-regular path %s\n' "$destination" >&2
        exit 1
    fi

    if [ -f "$destination" ] && digest=$(verify_model "$destination" "$name" "$size" "${sha256:--}" "${git_blob:--}"); then
        printf 'verified existing %s\n' "$name"
    else
        printf 'downloading %s\n' "$name"
        curl --fail --location --show-error --silent \
            --retry 3 --retry-all-errors --retry-delay 1 --connect-timeout 30 \
            --proto '=https' --proto-redir '=https' \
            --output "$staged.partial" "$url"
        digest=$(verify_model "$staged.partial" "$name" "$size" "${sha256:--}" "${git_blob:--}")
        mv "$staged.partial" "$staged"
    fi
    printf '%s  %s\n' "$digest" "$name" >> "$tmp_dir/checksums.sha256"
done <<'PINS'
face_detection_yunet_2023mar.onnx|232589|8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4|-|https://huggingface.co/opencv/face_detection_yunet/resolve/3cc26e7f1014a5ee5d74a42acee58bafc9d0a310/face_detection_yunet_2023mar.onnx
face_recognition_sface_2021dec.onnx|38696353|0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79|-|https://huggingface.co/opencv/face_recognition_sface/resolve/3d7082438a6e4551e840c9b2bb60b71e8da4b524/face_recognition_sface_2021dec.onnx
2.7_80x80_MiniFASNetV2.onnx|1744126|-|298096938aea527d6477d03a75de446df5cbcf75|https://raw.githubusercontent.com/QingHeYang/Silent-Face-Anti-Spoofing-onnx/584d4421d7ac42c59e640796f46e886b0095367a/onnx/2.7_80x80_MiniFASNetV2.onnx
4_0_0_80x80_MiniFASNetV1SE.onnx|1743294|-|69f1a09d7c076f72fd29c96a81ee88e0fba70134|https://raw.githubusercontent.com/QingHeYang/Silent-Face-Anti-Spoofing-onnx/584d4421d7ac42c59e640796f46e886b0095367a/onnx/4_0_0_80x80_MiniFASNetV1SE.onnx
PINS

# All downloads and reused files are verified before touching any installed model.
for name in \
    face_detection_yunet_2023mar.onnx \
    face_recognition_sface_2021dec.onnx \
    2.7_80x80_MiniFASNetV2.onnx \
    4_0_0_80x80_MiniFASNetV1SE.onnx
do
    if [ -f "$tmp_dir/$name" ]; then
        mv "$tmp_dir/$name" "$model_dir/$name"
    fi
done

mv -f "$tmp_dir/checksums.sha256" "$model_dir/checksums.sha256"
printf 'verified models installed in %s\n' "$model_dir"
