import re
import sys
import os

def parse_diff(file_path):
    with open(file_path, 'r', encoding='utf8') as f:
        lines = f.readlines()

    current_file = None
    current_line = 0
    current_fn = ""
    
    panic_paths = []
    unsafe_paths = []
    secrets_paths = []
    new_public_paths = []
    error_swallowing_paths = []

    re_panic = re.compile(r'(unwrap\(\)|expect\(|panic!|unreachable!|todo!|\[[a-z_][a-zA-Z0-9_]*\])')
    re_unsafe = re.compile(r'\bunsafe\b')
    re_secrets = re.compile(r'(password|secret|token|api_key|PRIVATE KEY|AKIA)', re.IGNORECASE)
    re_base64_hex = re.compile(r'["\'][a-zA-Z0-9+/=_-]{33,}["\']')
    re_ip = re.compile(r'\b(?!(?:127\.0\.0\.1|0\.0\.0\.0)\b)(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\b')
    re_pub = re.compile(r'\bpub\s+(fn|struct|enum)\s+[a-zA-Z0-9_]+.*')
    re_route = re.compile(r'\.route\(')
    re_error_swallow = re.compile(r'(let _ =|\.ok\(\);|unwrap_or_default\(\))')
    re_empty_err = re.compile(r'Err\([^)]*\)\s*=>\s*(\(\)|\{?\s*(?:(?:log::|tracing::|println!|eprintln!)[^;}]+;\s*)?\}?)')
    re_empty_catch = re.compile(r'catch\s*\{?\s*(?:(?:log::|tracing::|println!|eprintln!)[^;}]+;\s*)?\}?')

    for line in lines:
        if line.startswith('+++ b/'):
            current_file = line[6:].strip()
            continue
        
        if line.startswith('@@ '):
            m = re.match(r'@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)', line)
            if m:
                current_line = int(m.group(1)) - 1
                fn_context = m.group(2).strip()
                if fn_context:
                    current_fn = fn_context
                else:
                    current_fn = ""
            continue
            
        if line.startswith('+') and not line.startswith('+++'):
            current_line += 1
            if not current_file or not (current_file.startswith('core/src/') or current_file.startswith('cli/src/')):
                continue
                
            content = line[1:].strip()
            if not content:
                continue
            
            fn_display = current_fn if current_fn else "<unknown>"

            # 1. PANIC PATHS
            if re_panic.search(content):
                panic_paths.append(f"{current_file}:{current_line} (fn: {fn_display})")
                
            # 2. UNSAFE
            if re_unsafe.search(content):
                unsafe_paths.append(f"{current_file}:{current_line}")
                
            # 3. SECRETS
            if re_secrets.search(content) or re_base64_hex.search(content) or re_ip.search(content):
                secrets_paths.append(f"{current_file}:{current_line}")
                
            # 4. NEW PUBLIC SURFACE
            pub_match = re_pub.search(content)
            if pub_match:
                new_public_paths.append(f"{current_file}:{current_line} -> {pub_match.group(0)}")
            elif current_file.startswith('cli/src/') and re_route.search(content):
                new_public_paths.append(f"{current_file}:{current_line} -> {content}")
                
            # 5. ERROR SWALLOWING
            if re_error_swallow.search(content) or re_empty_err.search(content) or re_empty_catch.search(content):
                error_swallowing_paths.append(f"{current_file}:{current_line} (fn: {fn_display})")
                
        elif line.startswith('-') and not line.startswith('---'):
            pass
        elif not line.startswith('\\'):
            current_line += 1

    os.makedirs('docs/security', exist_ok=True)
    with open('docs/security/AGY_EVIDENCE_SWEEP_c3dae2de.md', 'w') as out:
        out.write("1. PANIC PATHS\n")
        if panic_paths:
            for p in panic_paths: out.write(f"{p}\n")
        else:
            out.write("NONE\n")
            
        out.write("\n2. UNSAFE\n")
        if unsafe_paths:
            for p in unsafe_paths: out.write(f"{p}\n")
        else:
            out.write("NONE\n")
            
        out.write("\n3. SECRETS\n")
        if secrets_paths:
            for p in secrets_paths: out.write(f"{p}\n")
        else:
            out.write("NONE\n")
            
        out.write("\n4. NEW PUBLIC SURFACE\n")
        if new_public_paths:
            for p in new_public_paths: out.write(f"{p}\n")
        else:
            out.write("NONE\n")
            
        out.write("\n5. ERROR SWALLOWING\n")
        if error_swallowing_paths:
            for p in error_swallowing_paths: out.write(f"{p}\n")
        else:
            out.write("NONE\n")

if __name__ == '__main__':
    parse_diff('tmp_diff.txt')
