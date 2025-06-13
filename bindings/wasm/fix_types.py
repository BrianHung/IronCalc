header = r"""
/* tslint:disable */
/* eslint-disable */
""".strip()

def fix_types(text: str):
    with open("types.ts") as f:
        types_str = f.read()
        header_types = "{}\n\n{}".format(header, types_str)
    
    text = text.replace(header, header_types)

    lines_with_any = []
    for line in text.splitlines():
        line = line.lstrip()
        # Skip internal methods
        if line.startswith("readonly model_"):
            continue
        if line.find("any") != -1:
            lines_with_any.append(line)

    if len(lines_with_any) > 0:
        print("There are 'unfixed' public types. Please check.\n", "\n".join(lines_with_any))
        exit(1)

    return text

if __name__ == "__main__":
    types_file = "pkg/ironcalc.d.ts"
    with open(types_file) as f:
        text = f.read()
    text = fix_types(text)
    with open(types_file, "wb") as f:
        f.write(bytes(text, "utf8"))
    
