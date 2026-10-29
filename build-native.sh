#!/bin/bash
# Build the native image for the Music AI Creator app
# Prerequisites:
#   - GraalVM with native-image installed
#   - Kotlin CLI (module.yaml build system)
#
# Usage:
#   ./build-native.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

APP_NAME="musicai-creator"
BUILD_DIR="build"
GRAAL_CONFIG_DIR="graal-config"
OUTPUT_DIR="build/native"
CACHE_DIR="$HOME/Library/Caches/JetBrains/Kotlin/.m2.cache"

echo "🔧 Building native image for $APP_NAME..."
echo ""

# Step 1: Build the project
echo "📦 Step 1: Building project..."
./kotlin build
echo "✅ Build successful"
echo ""

# Step 2: Construct classpath from dependency cache
echo "🔍 Step 2: Constructing classpath..."

# Helper: find the latest non-sources JAR for a given module group and artifact
# Usage: find_jar "org/jetbrains/kotlinx" "kotlinx-coroutines-core-jvm"
find_jar() {
  local group="$1"
  local artifact="$2"
  local group_path="$CACHE_DIR/$group"
  # Find the latest version directory, then the JAR
  local version=$(ls -1 "$group_path/$artifact/" 2>/dev/null | sort -V | tail -1)
  if [ -n "$version" ]; then
    local jar=$(ls "$group_path/$artifact/$version/$artifact-$version.jar" 2>/dev/null)
    if [ -n "$jar" ]; then
      echo "$jar"
      return
    fi
  fi
  # Fallback: search by name pattern within the group
  find "$group_path" -maxdepth 3 -name "$artifact-*.jar" ! -name "*sources*" -type f 2>/dev/null | sort -V | tail -1
}

# Project JAR
PROJECT_JAR="$BUILD_DIR/tasks/_musicai-creator_jarJvm/musicai-creator-jvm.jar"

# Find Ktor version dynamically
KTOR_DIR="$CACHE_DIR/io/ktor/ktor-server-core-jvm"
KTOR_VER=$(ls -1 "$KTOR_DIR" 2>/dev/null | sort -V | tail -1)
if [ -z "$KTOR_VER" ]; then
  echo "❌ Error: Could not find Ktor in cache"
  exit 1
fi
echo "   Found Ktor version: $KTOR_VER"

# Find Kotlin version dynamically (use the latest kotlin-stdlib directory)
KOTLIN_DIR="$CACHE_DIR/org/jetbrains/kotlin/kotlin-stdlib"
KOTLIN_VER=$(ls -1 "$KOTLIN_DIR" 2>/dev/null | sort -V | tail -1)
if [ -z "$KOTLIN_VER" ]; then
  echo "❌ Error: Could not find Kotlin in cache"
  exit 1
fi
echo "   Found Kotlin version: $KOTLIN_VER"

# Collect all required JARs using dynamic version discovery
JARS=(
  "$PROJECT_JAR"

  # Ktor server
  "$CACHE_DIR/io/ktor/ktor-server-core-jvm/$KTOR_VER/ktor-server-core-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-server-cio-jvm/$KTOR_VER/ktor-server-cio-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-server-config-yaml-jvm/$KTOR_VER/ktor-server-config-yaml-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-server-content-negotiation-jvm/$KTOR_VER/ktor-server-content-negotiation-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-server-cors-jvm/$KTOR_VER/ktor-server-cors-jvm-$KTOR_VER.jar"

  # Ktor client
  "$CACHE_DIR/io/ktor/ktor-client-core-jvm/$KTOR_VER/ktor-client-core-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-client-java-jvm/$KTOR_VER/ktor-client-java-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-client-content-negotiation-jvm/$KTOR_VER/ktor-client-content-negotiation-jvm-$KTOR_VER.jar"

  # Ktor serialization
  "$CACHE_DIR/io/ktor/ktor-serialization-kotlinx-json-jvm/$KTOR_VER/ktor-serialization-kotlinx-json-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-serialization-kotlinx-jvm/$KTOR_VER/ktor-serialization-kotlinx-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-serialization-jvm/$KTOR_VER/ktor-serialization-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-openapi-schema-jvm/$KTOR_VER/ktor-openapi-schema-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-server-routing-openapi-jvm/$KTOR_VER/ktor-server-routing-openapi-jvm-$KTOR_VER.jar"

  # Ktor core
  "$CACHE_DIR/io/ktor/ktor-http-jvm/$KTOR_VER/ktor-http-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-http-cio-jvm/$KTOR_VER/ktor-http-cio-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-io-jvm/$KTOR_VER/ktor-io-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-network-jvm/$KTOR_VER/ktor-network-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-utils-jvm/$KTOR_VER/ktor-utils-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-events-jvm/$KTOR_VER/ktor-events-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-sse-jvm/$KTOR_VER/ktor-sse-jvm-$KTOR_VER.jar"
  "$CACHE_DIR/io/ktor/ktor-websockets-jvm/$KTOR_VER/ktor-websockets-jvm-$KTOR_VER.jar"

  # Kotlin stdlib and reflect
  "$CACHE_DIR/org/jetbrains/kotlin/kotlin-stdlib/$KOTLIN_VER/kotlin-stdlib-$KOTLIN_VER.jar"
  "$(find_jar "org/jetbrains/kotlin" "kotlin-reflect")"

  # Kotlinx coroutines
  "$(find_jar "org/jetbrains/kotlinx" "kotlinx-coroutines-core-jvm")"
  "$(find_jar "org/jetbrains/kotlinx" "kotlinx-coroutines-jdk8")"
  "$(find_jar "org/jetbrains/kotlinx" "kotlinx-coroutines-slf4j")"

  # Kotlinx serialization
  "$(find_jar "org/jetbrains/kotlinx" "kotlinx-serialization-json-jvm")"
  "$(find_jar "org/jetbrains/kotlinx" "kotlinx-serialization-core-jvm")"
  "$(find_jar "org/jetbrains/kotlinx" "kotlinx-serialization-json-io-jvm")"

  # Kotlinx IO
  "$(find_jar "org/jetbrains/kotlinx" "kotlinx-io-core-jvm")"
  "$(find_jar "org/jetbrains/kotlinx" "kotlinx-io-bytestring-jvm")"

  # Logging
  "$(find_jar "ch/qos/logback" "logback-classic")"
  "$(find_jar "ch/qos/logback" "logback-core")"
  "$(find_jar "org/slf4j" "slf4j-api")"

  # Annotations
  "$(find_jar "org/jetbrains" "annotations")"

  # Typesafe config (Ktor server config)
  "$(find_jar "com/typesafe" "config")"

  # YAML parsing (for application.yaml) - Ktor 3.x uses kaml + yamlkt
  "$(find_jar "com/charleskorn/kaml" "kaml-jvm")"
  "$(find_jar "com/squareup/okio" "okio-jvm")"
  "$(find_jar "net/mamoe/yamlkt" "yamlkt")"
  "$(find_jar "it/krzeminski" "snakeyaml-engine-kmp-jvm")"
  "$(find_jar "net/thauvin/erik/urlencoder" "urlencoder-lib-jvm")"
)

# Build classpath string
CLASSPATH=""
JAR_COUNT=0
for jar in "${JARS[@]}"; do
  if [ -z "$jar" ] || [ ! -f "$jar" ]; then
    if [ -n "$jar" ]; then
      echo "⚠️  Warning: Missing JAR: $jar"
    fi
    continue
  fi
  JAR_COUNT=$((JAR_COUNT + 1))
  if [ -z "$CLASSPATH" ]; then
    CLASSPATH="$jar"
  else
    CLASSPATH="$CLASSPATH:$jar"
  fi
done

if [ -z "$CLASSPATH" ]; then
  echo "❌ Error: Could not construct classpath"
  exit 1
fi
echo "✅ Classpath constructed with $JAR_COUNT JARs"
echo ""

# Step 3: Create output directory
mkdir -p "$OUTPUT_DIR"

# Step 4: Build native image
echo "🔨 Step 3: Building native image with GraalVM..."
echo "   This may take several minutes..."
echo ""

native-image \
  --name="$APP_NAME" \
  -o "$OUTPUT_DIR/$APP_NAME" \
  -cp "$CLASSPATH" \
  -H:ReflectionConfigurationFiles="$GRAAL_CONFIG_DIR/reflect-config.json" \
  -H:ResourceConfigurationFiles="$GRAAL_CONFIG_DIR/resource-config.json" \
  -H:JNIConfigurationFiles="$GRAAL_CONFIG_DIR/jni-config.json" \
  -H:Class=org.jetbrains.amper.ktor.ApplicationKt \
  --initialize-at-build-time=io.ktor \
  --initialize-at-build-time=kotlinx.serialization \
  --initialize-at-build-time=kotlinx.coroutines \
  --initialize-at-build-time=kotlinx.io \
  --initialize-at-build-time=kotlin \
  --initialize-at-build-time=ch.qos.logback \
  --initialize-at-build-time=org.slf4j \
  -H:+ReportExceptionStackTraces \
  -R:MaxHeapSize=4g

echo ""
echo "🎉 Native image built successfully!"
echo "📍 Location: $OUTPUT_DIR/$APP_NAME"
echo ""
echo "Run it with:"
echo "  $OUTPUT_DIR/$APP_NAME"
echo ""
echo "Or set your API key and run:"
echo "  ACESTEP_API_KEY=your_key_here $OUTPUT_DIR/$APP_NAME"
