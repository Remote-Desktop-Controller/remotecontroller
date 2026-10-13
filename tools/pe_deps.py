"""Diagnóstico de dependências de um PE do Windows.

Lista as DLLs importadas por um executável e verifica, para cada função importada,
se ela existe na tabela de exportações da DLL correspondente encontrada no caminho
de busca padrão do carregador (diretório do exe, System32, CWD, PATH).

Uso: python pe_deps.py <caminho-do-exe>
"""

import os
import struct
import sys

IMAGE_ORDINAL_FLAG64 = 0x8000000000000000
IMAGE_ORDINAL_FLAG32 = 0x80000000


def read_pe(path):
    with open(path, "rb") as handle:
        data = handle.read()

    if data[:2] != b"MZ":
        raise ValueError("não é um PE válido (assinatura MZ ausente)")

    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe_offset : pe_offset + 4] != b"PE\0\0":
        raise ValueError("cabeçalho PE ausente")

    coff = pe_offset + 4
    machine, section_count, _, _, _, opt_size, _ = struct.unpack_from("<HHIIIHH", data, coff)
    opt_offset = coff + 20
    magic = struct.unpack_from("<H", data, opt_offset)[0]
    is_64 = magic == 0x20B

    directories_offset = opt_offset + (112 if is_64 else 96)
    import_rva, import_size = struct.unpack_from("<II", data, directories_offset + 8)

    sections = []
    section_offset = opt_offset + opt_size
    for index in range(section_count):
        base = section_offset + index * 40
        name = data[base : base + 8].rstrip(b"\0").decode("ascii", "replace")
        virtual_size, virtual_address, raw_size, raw_pointer = struct.unpack_from(
            "<IIII", data, base + 8
        )
        sections.append((name, virtual_address, virtual_size, raw_pointer, raw_size))

    def rva_to_offset(rva):
        for _, virtual_address, virtual_size, raw_pointer, raw_size in sections:
            if virtual_address <= rva < virtual_address + max(virtual_size, raw_size):
                return raw_pointer + (rva - virtual_address)
        return None

    def read_c_string(offset):
        end = data.index(b"\0", offset)
        return data[offset:end].decode("ascii", "replace")

    imports = {}
    descriptor = rva_to_offset(import_rva)
    while True:
        (original_first_thunk, _, _, name_rva, first_thunk) = struct.unpack_from(
            "<IIIII", data, descriptor
        )
        if name_rva == 0 and first_thunk == 0 and original_first_thunk == 0:
            break

        dll_name = read_c_string(rva_to_offset(name_rva))
        thunk_rva = original_first_thunk or first_thunk
        thunk_offset = rva_to_offset(thunk_rva)
        names = []
        step = 8 if is_64 else 4
        ordinal_flag = IMAGE_ORDINAL_FLAG64 if is_64 else IMAGE_ORDINAL_FLAG32

        while True:
            value = struct.unpack_from("<Q" if is_64 else "<I", data, thunk_offset)[0]
            if value == 0:
                break
            if value & ordinal_flag:
                names.append(("ordinal", value & 0xFFFF))
            else:
                hint_name_offset = rva_to_offset(value)
                struct.unpack_from("<H", data, hint_name_offset)[0]
                names.append(("name", read_c_string(hint_name_offset + 2)))
            thunk_offset += step

        imports[dll_name] = names
        descriptor += 20

    return imports, is_64


def list_exports(path):
    with open(path, "rb") as handle:
        data = handle.read()

    pe_offset = struct.unpack_from("<I", data, 0x3C)[0]
    coff = pe_offset + 4
    machine, section_count, _, _, _, opt_size, _ = struct.unpack_from("<HHIIIHH", data, coff)
    opt_offset = coff + 20
    magic = struct.unpack_from("<H", data, opt_offset)[0]
    is_64 = magic == 0x20B

    directories_offset = opt_offset + (112 if is_64 else 96)
    export_rva, export_size = struct.unpack_from("<II", data, directories_offset)

    sections = []
    section_offset = opt_offset + opt_size
    for index in range(section_count):
        base = section_offset + index * 40
        virtual_size, virtual_address, raw_size, raw_pointer = struct.unpack_from(
            "<IIII", data, base + 8
        )
        sections.append((virtual_address, virtual_size, raw_pointer, raw_size))

    def rva_to_offset(rva):
        for virtual_address, virtual_size, raw_pointer, raw_size in sections:
            if virtual_address <= rva < virtual_address + max(virtual_size, raw_size):
                return raw_pointer + (rva - virtual_address)
        return None

    if export_rva == 0 or export_size == 0:
        return set(), set(), False, is_64

    export_offset = rva_to_offset(export_rva)
    (
        _name_rva,
        ordinal_base,
        function_count,
        name_count,
        functions_rva,
        names_rva,
        ordinals_rva,
    ) = struct.unpack_from("<IIIIIII", data, export_offset + 12)

    names = set()
    names_offset = rva_to_offset(names_rva)
    for index in range(name_count):
        name_rva = struct.unpack_from("<I", data, names_offset + index * 4)[0]
        string_offset = rva_to_offset(name_rva)
        end = data.index(b"\0", string_offset)
        names.add(data[string_offset:end].decode("ascii", "replace"))

    ordinals = set()
    ordinal_offset = rva_to_offset(ordinals_rva)
    if ordinal_offset and function_count:
        for index in range(min(function_count, name_count or function_count)):
            value = struct.unpack_from("<H", data, ordinal_offset + index * 2)[0]
            ordinals.add(ordinal_base + value)

    return names, ordinals, True, is_64


def resolve(dll_name, exe_dir):
    candidates = [
        os.path.join(exe_dir, dll_name),
        os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32", dll_name),
        os.path.join(os.getcwd(), dll_name),
    ]
    for entry in os.environ.get("PATH", "").split(os.pathsep):
        if entry:
            candidates.append(os.path.join(entry, dll_name))

    for candidate in candidates:
        if os.path.isfile(candidate):
            return candidate
    return None


def main():
    exe = sys.argv[1]
    exe_dir = os.path.dirname(os.path.abspath(exe))
    imports, is_64 = read_pe(exe)
    print(f"Executável: {exe} ({'x64' if is_64 else 'x86'})")
    print(f"DLLs importadas: {len(imports)}\n")

    problems = []
    for dll_name, symbols in sorted(imports.items()):
        resolved = resolve(dll_name, exe_dir)
        named = [value for kind, value in symbols if kind == "name"]
        if resolved is None:
            print(f"[AUSENTE] {dll_name}: não encontrada no caminho de busca ({len(named)} importações)")
            problems.append(dll_name)
            continue

        try:
            exports, ordinals, has_exports, dll_is_64 = list_exports(resolved)
        except Exception as error:  # pragma: no cover - diagnóstico
            print(f"[ERRO] {dll_name} em {resolved}: {error}")
            continue

        missing = [name for name in named if name not in exports]
        arch_note = "" if dll_is_64 == is_64 else " (ARQUITETURA DIFERENTE!)"
        if missing:
            print(f"[FALTANDO] {dll_name} em {resolved}{arch_note}: {len(missing)} export(s) ausente(s)")
            for name in missing[:15]:
                print(f"    - {name}")
            problems.append(dll_name)
        else:
            print(f"[OK] {dll_name} em {resolved}: {len(named)} importações satisfeitas{arch_note}")

    print()
    if problems:
        print(f"Dependências problemáticas: {', '.join(sorted(set(problems)))}")
        return 1
    print("Todas as importações estáticas foram satisfeitas.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
