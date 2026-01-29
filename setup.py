import os
import sys
import shutil
from pathlib import Path
from setuptools import setup, find_packages, Extension
from setuptools.command.build_ext import build_ext

CMAKE_PATH = shutil.which("cmake") or "cmake"
C_COMPILER_PATH = shutil.which("gcc") or "gcc"
CXX_COMPILER_PATH = shutil.which("g++") or "g++"
ENGINE_SOURCE_DIR = "src/"


class CMakeBuildExtension(build_ext):
    """Custom CMake build extension that builds AGFS and C++ extensions."""

    def run(self):
        self.build_agfs()
        self.cmake_executable = CMAKE_PATH

        for ext in self.extensions:
            self.build_extension(ext)

    def build_agfs(self):
        """Build AGFS server, fallback to prebuilt binary if build fails."""
        agfs_server_dir = Path("third_party/agfs/agfs-server").resolve()
        binary_name = "agfs-server.exe" if sys.platform == "win32" else "agfs-server"
        agfs_prebuilt_binary = Path(f"third_party/agfs/bin/{binary_name}").resolve()
        agfs_bin_dir = Path("openviking/bin").resolve()
        agfs_target_binary = agfs_bin_dir / binary_name

        agfs_bin_dir.mkdir(parents=True, exist_ok=True)

        build_success = False
        if agfs_server_dir.exists():
            go_executable = shutil.which("go")
            if go_executable:
                print("Building AGFS server from source...")
                import subprocess
                try:
                    if sys.platform == "win32":
                        # Now that we fixed the code compatibility, we can use go build directly on Windows
                        build_cmd = ["go", "build", "-o", f"build/{binary_name}", "cmd/server/main.go"]
                    else:
                        build_cmd = ["make", "build"]

                    subprocess.run(
                        build_cmd,
                        cwd=str(agfs_server_dir),
                        check=True,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE
                    )

                    agfs_built_binary = agfs_server_dir / "build" / binary_name
                    if agfs_built_binary.exists():
                        shutil.copy2(str(agfs_built_binary), str(agfs_target_binary))
                        if sys.platform != "win32":
                            os.chmod(str(agfs_target_binary), 0o755)
                        print(f"[OK] AGFS server built successfully from source")
                        build_success = True
                except subprocess.CalledProcessError as e:
                    print(f"Warning: Failed to build AGFS from source: {e}")
                    if e.stdout:
                        print(f"Build stdout:\n{e.stdout.decode('utf-8', errors='replace')}")
                    if e.stderr:
                        print(f"Build stderr:\n{e.stderr.decode('utf-8', errors='replace')}")
                except Exception as e:
                    print(f"Warning: Failed to build AGFS from source: {e}")
            else:
                print("Warning: Go compiler not found, will use prebuilt binary")

        if not build_success:
            if agfs_prebuilt_binary.exists():
                print(f"Using prebuilt AGFS binary from {agfs_prebuilt_binary}")
                shutil.copy2(str(agfs_prebuilt_binary), str(agfs_target_binary))
                if sys.platform != "win32":
                    os.chmod(str(agfs_target_binary), 0o755)
                print(f"[OK] AGFS server copied from prebuilt binary")
            else:
                print("Error: No AGFS binary available (build failed and no prebuilt binary found)")
                raise FileNotFoundError(
                    f"AGFS binary not available. Please either:\n"
                    f"  1. Install Go and build from source, or\n"
                    f"  2. Ensure prebuilt binary exists at third_party/agfs/bin/{binary_name}"
                )

    def build_extension(self, ext):
        """Build a single C++ extension module using CMake."""
        ext_fullpath = Path(self.get_ext_fullpath(ext.name))
        ext_dir = ext_fullpath.parent.resolve()
        build_dir = Path(self.build_temp) / "cmake_build"
        build_dir.mkdir(parents=True, exist_ok=True)

        cmake_args = [
            f"-S{Path(ENGINE_SOURCE_DIR).resolve()}",
            f"-B{build_dir}",
            f"-DCMAKE_BUILD_TYPE=Release",
            f"-DPY_OUTPUT_DIR={ext_dir}",
            "-DCMAKE_VERBOSE_MAKEFILE=ON",
            "-DCMAKE_INSTALL_RPATH=$ORIGIN",
            f"-DPython3_EXECUTABLE={sys.executable}",
            f"-DPython3_INCLUDE_DIRS={sysconfig.get_path('include')}",
            f"-DPython3_LIBRARIES={sysconfig.get_config_vars().get('LIBRARY')}",
            f"-Dpybind11_DIR={pybind11.get_cmake_dir()}",
            f"-DCMAKE_C_COMPILER={C_COMPILER_PATH}",
            f"-DCMAKE_CXX_COMPILER={CXX_COMPILER_PATH}",
        ]

        if sys.platform == "darwin":
            cmake_args.append("-DCMAKE_OSX_DEPLOYMENT_TARGET=10.15")
        elif sys.platform == "win32":
            cmake_args.extend(["-G", "MinGW Makefiles"])

        self.spawn([self.cmake_executable] + cmake_args)

        build_args = [
            "--build", str(build_dir),
            "--config", "Release",
            f"-j{os.cpu_count() or 4}"
        ]
        self.spawn([self.cmake_executable] + build_args)


import sysconfig
import pybind11

setup(
    ext_modules=[
        Extension(
            name="openviking.storage.vectordb.engine",
            sources=[],
        )
    ],
    cmdclass={
        "build_ext": CMakeBuildExtension,
    },
    packages=find_packages(),
    package_data={
        "openviking": [
            "bin/agfs-server",
            "bin/agfs-server.exe",
        ],
    },
    include_package_data=True,
)
