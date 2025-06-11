def check_types(text: str) -> None:
    """Ensure generated definitions don't contain any stray 'any' types."""
    if text.find("any") != -1:
        print("There are 'unfixed' types. Please check.")
        exit(1)

def fix_types(ts_file: str) -> None:
    """Validate generated TypeScript definitions."""
    with open(ts_file) as f:
        text = f.read()
    check_types(text)

def append_js(js_file: str) -> None:
    """Prepend the generated TypeScript enums JS to the wasm bundle."""
    with open("types.js") as f:
        text_js = f.read()
    with open(js_file) as f:
        text = f.read()

    with open(js_file, "wb") as f:
        f.write(bytes("{}\n{}".format(text_js, text), "utf8"))

if __name__ == "__main__":
    ts_file = "pkg/wasm.d.ts"
    fix_types(ts_file)

    js_file = "pkg/wasm.js"
    append_js(js_file)
    