#!/usr/bin/env python3
"""Unit and Integration Tests for SCMessenger Wiring & Reachability Gate (scripts/check_wiring.py)."""

import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

from scripts.check_wiring import (
    Declaration,
    Finding,
    check_nav_routes,
    check_wiring,
    extract_declarations,
    parse_manifest,
    strip_comments,
)


class TestWiringGate(unittest.TestCase):
    """Test suite for check_wiring static analysis and reachability gate."""

    def test_strip_comments(self):
        code = """
        // Single line comment with ClassName
        /* Multi-line comment
           with OtherClass */
        val x = 10 // trailing comment
        val str = "Hello // not a comment"
        val raw = \"\"\"
            /* not a comment inside raw string */
        \"\"\"
        class LiveClass
        """
        stripped = strip_comments(code)
        self.assertNotIn("ClassName", stripped)
        self.assertNotIn("OtherClass", stripped)
        self.assertIn("LiveClass", stripped)
        self.assertIn("Hello // not a comment", stripped)

    def test_manifest_missing_c3(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            main_dir = os.path.join(tmpdir, "android", "app", "src", "main")
            java_dir = os.path.join(main_dir, "java", "com", "test")
            os.makedirs(java_dir, exist_ok=True)

            manifest_content = """<?xml version="1.0" encoding="utf-8"?>
            <manifest xmlns:android="http://schemas.android.com/apk/res/android">
                <application android:name=".TestApp">
                    <activity android:name=".MainActivity" />
                </application>
            </manifest>
            """
            with open(os.path.join(main_dir, "AndroidManifest.xml"), "w", encoding="utf-8") as fh:
                fh.write(manifest_content)

            service_code = """package com.test
            import android.app.Service
            class OrphanService : Service()
            """
            with open(os.path.join(java_dir, "OrphanService.kt"), "w", encoding="utf-8") as fh:
                fh.write(service_code)

            findings, _ = check_wiring(tmpdir)
            c3_findings = [f for f in findings if f.kind == "C3_MANIFEST_MISSING"]
            self.assertTrue(any(f.symbol == "OrphanService" for f in c3_findings))

    def test_nav_route_unregistered_c2(self):
        mesh_app_code = """
        sealed class Screen(val route: String) {
            object Live : Screen("live")
            object Dead : Screen("dead")
        }

        @Composable
        fun MeshNavHost(navController: NavHostController) {
            NavHost(navController = navController, startDestination = Screen.Live.route) {
                composable(Screen.Live.route) {
                    LiveScreen(onNavigate = { navController.navigate(Screen.Dead.route) })
                }
            }
        }
        """
        findings, registered, _ = check_nav_routes("MeshApp.kt", mesh_app_code, ".")
        c2_findings = [f for f in findings if f.kind == "C2_UNREGISTERED_ROUTE"]
        self.assertEqual(len(c2_findings), 1)
        self.assertIn("Screen.Dead", c2_findings[0].symbol)

    def test_preview_exclusion(self):
        kt_files = {
            "TestPreview.kt": """package com.test
            import androidx.compose.runtime.Composable
            import androidx.compose.ui.tooling.preview.Preview

            @Preview
            @Composable
            fun PreviewScreen() {}
            """
        }
        clean_files = {k: strip_comments(v) for k, v in kt_files.items()}
        decls = extract_declarations(kt_files, clean_files)
        preview_decls = [d for d in decls if d.name == "PreviewScreen"]
        self.assertEqual(len(preview_decls), 1)
        self.assertTrue(preview_decls[0].is_preview)

    def test_zero_callers_c1(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            main_dir = os.path.join(tmpdir, "android", "app", "src", "main")
            java_dir = os.path.join(main_dir, "java", "com", "test")
            os.makedirs(java_dir, exist_ok=True)

            manifest_content = """<?xml version="1.0" encoding="utf-8"?>
            <manifest xmlns:android="http://schemas.android.com/apk/res/android">
                <application android:name=".TestApp">
                    <activity android:name=".MainActivity" />
                </application>
            </manifest>
            """
            with open(os.path.join(main_dir, "AndroidManifest.xml"), "w", encoding="utf-8") as fh:
                fh.write(manifest_content)

            app_code = """package com.test
            import android.app.Activity
            class MainActivity : Activity()
            """
            with open(os.path.join(java_dir, "MainActivity.kt"), "w", encoding="utf-8") as fh:
                fh.write(app_code)

            orphan_code = """package com.test
            import androidx.compose.runtime.Composable
            @Composable
            fun OrphanDialog() {}
            """
            with open(os.path.join(java_dir, "OrphanDialog.kt"), "w", encoding="utf-8") as fh:
                fh.write(orphan_code)

            findings, _ = check_wiring(tmpdir)
            c1_findings = [f for f in findings if f.kind == "C1_ZERO_CALLERS"]
            self.assertTrue(any(f.symbol == "OrphanDialog" for f in c1_findings))

    def test_transitive_dead_c4(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            main_dir = os.path.join(tmpdir, "android", "app", "src", "main")
            java_dir = os.path.join(main_dir, "java", "com", "test")
            os.makedirs(java_dir, exist_ok=True)

            manifest_content = """<?xml version="1.0" encoding="utf-8"?>
            <manifest xmlns:android="http://schemas.android.com/apk/res/android">
                <application android:name=".TestApp">
                    <activity android:name=".MainActivity" />
                </application>
            </manifest>
            """
            with open(os.path.join(main_dir, "AndroidManifest.xml"), "w", encoding="utf-8") as fh:
                fh.write(manifest_content)

            app_code = """package com.test
            import android.app.Activity
            class MainActivity : Activity()
            """
            with open(os.path.join(java_dir, "MainActivity.kt"), "w", encoding="utf-8") as fh:
                fh.write(app_code)

            dead_code = """package com.test
            import androidx.compose.runtime.Composable
            @Composable
            fun DeadCaller() {
                ChildDialog()
            }
            @Composable
            fun ChildDialog() {}
            """
            with open(os.path.join(java_dir, "DeadFeature.kt"), "w", encoding="utf-8") as fh:
                fh.write(dead_code)

            findings, _ = check_wiring(tmpdir)
            c1_symbols = {f.symbol for f in findings if f.kind == "C1_ZERO_CALLERS"}
            c4_symbols = {f.symbol for f in findings if f.kind == "C4_TRANSITIVE_DEAD"}
            self.assertIn("DeadCaller", c1_symbols)
            self.assertIn("ChildDialog", c4_symbols)

    def test_real_repo_clean_wiring(self):
        """Verify that running against the repository finds zero wiring defects on this branch."""
        repo_root = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
        findings, _ = check_wiring(repo_root)
        self.assertEqual(
            len(findings),
            0,
            f"Expected 0 wiring findings on branch fix/android-restore-wiring, got {len(findings)}: {[f.symbol for f in findings]}"
        )


if __name__ == "__main__":
    unittest.main()
