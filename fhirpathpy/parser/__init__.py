try:
    from fhirpathpy._rust import IMPLEMENTED as _rust_ready
    from fhirpathpy._rust import parse as _rust_parse

    if not _rust_ready:
        raise ImportError("Rust parser not yet implemented")
    parse = _rust_parse
except ImportError:
    from fhirpathpy.parser._antlr import parse  # noqa: F401
