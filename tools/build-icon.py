from pathlib import Path

from PIL import Image


PROJECT_ROOT = Path(__file__).resolve().parents[1]
SOURCE = PROJECT_ROOT / "assets" / "bluetooth-battery-icon.png"
SQUARE_MASTER = PROJECT_ROOT / "assets" / "bluetooth-battery-icon-square.png"
ICO_OUTPUT = PROJECT_ROOT / "icon.ico"
CANVAS_SIZE = 1024
SUBJECT_MAX = 896
ICO_SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)


def main() -> None:
    with Image.open(SOURCE) as opened:
        source = opened.convert("RGBA")

    bounds = source.getchannel("A").getbbox()
    if bounds is None:
        raise RuntimeError(f"No visible pixels found in {SOURCE}")

    subject = source.crop(bounds)
    scale = min(SUBJECT_MAX / subject.width, SUBJECT_MAX / subject.height)
    size = (
        max(1, round(subject.width * scale)),
        max(1, round(subject.height * scale)),
    )
    subject = subject.resize(size, Image.Resampling.LANCZOS)

    canvas = Image.new("RGBA", (CANVAS_SIZE, CANVAS_SIZE), (0, 0, 0, 0))
    position = (
        (CANVAS_SIZE - subject.width) // 2,
        (CANVAS_SIZE - subject.height) // 2,
    )
    canvas.alpha_composite(subject, position)

    canvas.save(SQUARE_MASTER, format="PNG", optimize=True)
    canvas.save(
        ICO_OUTPUT,
        format="ICO",
        sizes=[(size, size) for size in ICO_SIZES],
    )

    print(f"Wrote {SQUARE_MASTER}")
    print(f"Wrote {ICO_OUTPUT}")
    print("ICO sizes: " + ", ".join(f"{size}x{size}" for size in ICO_SIZES))


if __name__ == "__main__":
    main()
