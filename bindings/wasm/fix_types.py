# This script runs after wasm-bindgen has generated `pkg/wasm.d.ts` and
# `pkg/wasm.js`.  Its job is now very small: make sure that the generated
# TypeScript definitions do not contain the `any` type and prepend the
# runtime enums compiled from `types.ts` (stored in `types.js`) to
# `pkg/wasm.js`.

def check_types(text: str) -> None:
    if "any" in text:
        print("There are 'any' types. Please check.")
        exit(1)
    


if __name__ == "__main__":
    types_file = "pkg/wasm.d.ts"
    with open(types_file) as f:
        text = f.read()
    check_types(text)

    js_file = "pkg/wasm.js"
    with open("types.js") as f:
        text_js = f.read()
    with open(js_file) as f:
        text = f.read()

    with open(js_file, "wb") as f:
        f.write(bytes(f"{text_js}\n{text}", "utf8"))
    

    
