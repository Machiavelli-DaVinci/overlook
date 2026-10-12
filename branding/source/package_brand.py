"""Package branding assets and llm-docs with portable relative links."""
from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED

PROJECT = Path(__file__).resolve().parents[2]
ARCHIVE = PROJECT / "branding/overlook-brand-kit.zip"
STAGING = ARCHIVE.with_suffix(".zip.tmp")

try:
    with ZipFile(STAGING, "w", ZIP_DEFLATED) as bundle:
        for folder in ("branding", "llm-docs"):
            for path in sorted((PROJECT / folder).rglob("*")):
                if not path.is_file() or path in (ARCHIVE, STAGING):
                    continue
                if "__pycache__" in path.parts or path.suffix == ".pyc":
                    continue
                bundle.write(path, Path("overlook-brand-kit") / path.relative_to(PROJECT))
    with ZipFile(STAGING) as bundle:
        bad_file = bundle.testzip()
        if bad_file:
            raise RuntimeError(f"Archive verification failed: {bad_file}")
        count = len(bundle.namelist())
    STAGING.replace(ARCHIVE)
except BaseException:
    STAGING.unlink(missing_ok=True)
    raise

print(f"Packaged and verified {count} files: {ARCHIVE}")
