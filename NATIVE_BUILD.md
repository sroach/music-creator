# GraalVM Native Image Build Guide

This project can be compiled to a **native executable** using GraalVM's `native-image` tool, producing a single binary with:
- **Faster startup**: ~100ms vs ~2 seconds for JVM
- **Lower memory**: ~30MB vs ~200MB+ for JVM  
- **Single executable**: No JDK required on the deployment target

## Prerequisites

### 1. GraalVM with native-image

Install GraalVM JDK 21+ with the `native-image` component:

```bash
# Using Homebrew (macOS)
brew install --cask graalvm/tap/graalvm

# Or using SDKMAN
sdk install java 21.0.2-graalce
sdk use java 21.0.2-graalce

# Verify installation
native-image --version
```

### 2. Kotlin CLI

The project uses the Kotlin `module.yaml` build system. Ensure the `kotlin` command is available:

```bash
kotlin --version
```

## Quick Start

### Build the native image:

```bash
./build-native.sh
```

This will:
1. Package the project as an executable JAR with all dependencies
2. Build the native image using GraalVM

The resulting binary will be at: `build/native/musicai-creator`

### Run the native executable:

```bash
# Set your API key and run
ACESTEP_API_KEY=your_key_here ./build/native/musicai-creator
```

The server will start at `http://localhost:8080`.

## Regenerating Configuration

If you add new dependencies or features that use reflection, regenerate the GraalVM configuration:

```bash
./generate-graal-config.sh
```

This runs the app with the GraalVM tracing agent to automatically detect all reflective calls, then generates updated configuration files in `graal-config-generated/`.

Merge the generated files into `graal-config/` or use them directly with the `native-image` command.

## Configuration Files

All GraalVM configuration files are in the `graal-config/` directory:

| File | Purpose |
|------|---------|
| `reflect-config.json` | Classes and methods accessed via reflection (Ktor, serialization, logging) |
| `resource-config.json` | Resources bundled into the native image (HTML, YAML, properties) |
| `jni-config.json` | JNI access requirements |

## Manual Build Command

For fine-grained control, build manually:

```bash
# Build the project
./kotlin build

# Build native image (classpath constructed from dependency cache)
# See build-native.sh for the full classpath construction
native-image \
  --name=musicai-creator \
  -o build/native/musicai-creator \
  -cp "$CLASSPATH" \
  -H:ReflectionConfigurationFiles=graal-config/reflect-config.json \
  -H:ResourceConfigurationFiles=graal-config/resource-config.json \
  -H:JNIConfigurationFiles=graal-config/jni-config.json \
  -H:Class=org.jetbrains.amper.ktor.ApplicationKt \
  --initialize-at-build-time=io.ktor \
  --initialize-at-build-time=kotlinx.serialization \
  --initialize-at-build-time=kotlin \
  --initialize-at-build-time=ch.qos.logback \
  --initialize-at-build-time=org.slf4j \
  -H:+ReportExceptionStackTraces \
  -R:MaxHeapSize=4g
```

## Troubleshooting

### "Class not found" at runtime

Add the missing class to `graal-config/reflect-config.json`:

```json
{
  "name": "com.example.MissingClass",
  "allDeclaredMethods": true,
  "allDeclaredFields": true
}
```

### "Resource not found" at runtime

Add the resource pattern to `graal-config/resource-config.json`:

```json
{
  "pattern": ".*missing-resource\\.ext"
}
```

### Out of memory during build

Increase the heap size in the build command:

```bash
-R:MaxHeapSize=8g
```

### Build takes too long

Reduce optimization level (faster build, larger binary):

```bash
-O2  # Instead of default -O3
```

## Docker Deployment

Create a minimal Docker image:

```dockerfile
FROM scratch
COPY build/native/musicai-creator /app
COPY resources/ /resources
EXPOSE 8080
CMD ["/app"]
```

Build and run:

```bash
docker build -t musicai-creator-native .
docker run -p 8080:8080 -e ACESTEP_API_KEY=your_key musicai-creator-native
```

The resulting image is typically **~50MB** vs **~300MB+** for a JVM-based image.
