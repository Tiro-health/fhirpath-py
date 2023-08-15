import base64
import re
import fhirpathpy.engine.util as util


def ensure_string_singleton(x):
    if len(x) == 1:
        d = util.get_data(x[0])
        if type(d) == str:
            return d
        raise Exception("Expected string, but got " + str(d))

    raise Exception("Expected string, but got " + str(x))


def index_of(ctx, coll, substr):
    string = ensure_string_singleton(coll)
    return string.find(substr)


def substring(ctx, coll, start, length=None):
    string = ensure_string_singleton(coll)
    start = int(start)

    if length is None:
        return string[start:]

    length = int(length)
    return string[start : start + length]


def starts_with(ctx, coll, prefix):
    string = ensure_string_singleton(coll)
    return string.startswith(prefix)


def ends_with(ctx, coll, postfix):
    string = ensure_string_singleton(coll)
    return string.endswith(postfix)


def contains_fn(ctx, coll, substr):
    string = ensure_string_singleton(coll)
    return substr in string


def upper(ctx, coll):
    string = ensure_string_singleton(coll)
    return string.upper()


def lower(ctx, coll):
    string = ensure_string_singleton(coll)
    return string.lower()


def split(ctx, coll, delimiter):
    string = ensure_string_singleton(coll)
    return string.split(delimiter)


def trim(ctx, coll):
    string = ensure_string_singleton(coll)
    return string.strip()


def encode(ctx, coll, format):
    if not coll:
        return []

    str_to_encode = coll[0] if isinstance(coll, list) else coll
    if not str_to_encode:
        return []

    if format in ["urlbase64", "base64url"]:
        encoded = base64.b64encode(str_to_encode.encode()).decode()
        return encoded.replace("+", "-").replace("/", "_")

    if format == "base64":
        return base64.b64encode(str_to_encode.encode()).decode()

    if format == "hex":
        return "".join([hex(ord(c))[2:].zfill(2) for c in str_to_encode])

    return []


def decode(ctx, coll, format):
    if not coll:
        return []

    str_to_decode = coll[0] if isinstance(coll, list) else coll
    if not str_to_decode:
        return []

    if format in ["urlbase64", "base64url"]:
        decoded = str_to_decode.replace("-", "+").replace("_", "/")
        return base64.b64decode(decoded).decode()

    if format == "base64":
        return base64.b64decode(str_to_decode).decode()

    if format == "hex":
        if len(str_to_decode) % 2 != 0:
            raise ValueError("Decode 'hex' requires an even number of characters.")
        return "".join(
            [chr(int(str_to_decode[i : i + 2], 16)) for i in range(0, len(str_to_decode), 2)]
        )

    return []


def join(ctx, coll, separator=""):
    stringValues = []
    for n in coll:
        d = util.get_data(n)
        if isinstance(d, str):
            stringValues.append(d)
        else:
            raise TypeError("Join requires a collection of strings.")

    return separator.join(stringValues)


# test function
def matches(ctx, coll, regex):
    string = ensure_string_singleton(coll)
    valid = re.compile(regex)
    return re.search(valid, string) is not None


def replace(ctx, coll, regex, repl):
    string = ensure_string_singleton(coll)
    return string.replace(regex, repl)


def replace_matches(ctx, coll, regex, repl):
    string = ensure_string_singleton(coll)
    valid = re.compile(regex)
    return re.sub(valid, repl, string)


def length(ctx, coll):
    str = ensure_string_singleton(coll)
    return len(str)


def toChars(ctx, coll):
    if not coll:
        return []
    string = ensure_string_singleton(coll)
    return list(string)
