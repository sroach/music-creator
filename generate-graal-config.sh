#!/bin/bash
# Generate GraalVM reflection configuration using the tracing agent
# This script runs the app with the native-image-agent to trace all reflective calls,
# then generates the configuration files in graal-config-generated/
#
# Prerequisites:
#   - GraalVM with native-image installed
#   - ACESTEP_API_KEY set (optional, for full tracing with a real API call)
#
# Usage:
#   ./generate-graal-config.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

AGENT_OUTPUT_DIR="graal-config-generated"

echo "🔧 Generating GraalVM configuration using tracing agent..."
echo ""

# Step 1: Build the project
echo "📦 Step 1: Building project..."
./kotlin build
echo "✅ Build successful"
echo ""

# Step 2: Clean previous agent output
echo "🧹 Step 2: Cleaning previous agent output..."
rm -rf "$AGENT_OUTPUT_DIR"
mkdir -p "$AGENT_OUTPUT_DIR"
echo ""

# Step 3: Run with the tracing agent
echo "🔍 Step 3: Running app with native-image-agent..."
echo "   The agent will trace all reflective calls made during startup."
echo "   The app will be automatically stopped after 10 seconds."
echo ""

# Build classpath (same as build-native.sh)
CACHE_DIR="$HOME/Library/Caches/JetBrains/Kotlin/.m2.cache"
PROJECT_JAR="build/tasks/_musicai-creator_jarJvm/musicai-creator-jvm.jar"
KTOR="io/ktor"
K="3.6.0"
KOTLIN_VER="2.4.20"

JARS=(
  "$PROJECT_JAR"
  "$CACHE_DIR/$KTOR/ktor-server-core-jvm/$K/ktor-server-core-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-server-cio-jvm/$K/ktor-server-cio-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-server-config-yaml-jvm/$K/ktor-server-config-yaml-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-server-content-negotiation-jvm/$K/ktor-server-content-negotiation-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-server-cors-jvm/$K/ktor-server-cors-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-client-core-jvm/$K/ktor-client-core-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-client-java-jvm/$K/ktor-client-java-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-client-content-negotiation-jvm/$K/ktor-client-content-negotiation-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-serialization-kotlinx-json-jvm/$K/ktor-serialization-kotlinx-json-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-serialization-kotlinx-jvm/$K/ktor-serialization-kotlinx-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-serialization-jvm/$K/ktor-serialization-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-http-jvm/$K/ktor-http-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-http-cio-jvm/$K/ktor-http-cio-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-io-jvm/$K/ktor-io-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-network-jvm/$K/ktor-network-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-utils-jvm/$K/ktor-utils-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-events-jvm/$K/ktor-events-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-sse-jvm/$K/ktor-sse-jvm-$K.jar"
  "$CACHE_DIR/$KTOR/ktor-websockets-jvm/$K/ktor-websockets-jvm-$K.jar"
  "$CACHE_DIR/org/jetbrains/kotlin/kotlin-stdlib/$KOTLIN_VER/kotlin-stdlib-$KOTLIN_VER.jar"
  "$CACHE_DIR/org/jetbrains/kotlin/kotlin-reflect/$KOTLIN_VER/kotlin-reflect-$KOTLIN_VER.jar"
  "$CACHE_DIR/org/jetbrains/kotlinx/kotlinx-coroutines-core-jvm/1.10.1/kotlinx-coroutines-core-jvm-1.10.1.jar"
  "$CACHE_DIR/org/jetbrains/kotlinx/kotlinx-serialization-json-jvm/1.8.0/kotlinx-serialization-json-jvm-1.8.0.jar"
  "$CACHE_DIR/org/jetbrains/kotlinx/kotlinx-serialization-core-jvm/1.8.0/kotlinx-serialization-core-jvm-1.8.0.jar"
  "$CACHE_DIR/ch/qos/logback/logback-classic/1.5.38/logback-classic-1.5.38.jar"
  "$CACHE_DIR/ch/qos/logback/logback-core/1.5.38/logback-core-1.5.38.jar"
  "$CACHE_DIR/org/slf4j/slf4j-api/2.0.17/slf4j-api-2.0.17.jar"
  "$CACHE_DIR/org/yaml/snakeyaml/2.3/snakeyaml-2.3.jar"
)

CLASSPATH=""
for jar in "${JARS[@]}"; do
  if [ -f "$jar" ]; then
    if [ -z "$CLASSPATH" ]; then
      CLASSPATH="$jar"
    else
      CLASSPATH="$CLASSPATH:$jar"
    fi
  fi
done

# Run with the agent
java \
  -agentlib:native-image-agent=config-output-dir="$AGENT_OUTPUT_DIR" \
  -cp "$CLASSPATH" \
  org.jetbrains.amper.ktor.ApplicationKt \
  &

JAVA_PID=$!

echo "⏳ Waiting for server to start (PID: $JAVA_PID)..."
sleep 10

# Make a test request to trigger more reflective paths
echo "🌐 Making test request to trigger reflective paths..."
curl -s http://localhost:8080/ > /dev/null 2>&1 || true

# Stop the app
echo "🛑 Stopping the app..."
kill $JAVA_PID 2>/dev/null || true
wait $JAVA_PID 2>/dev/null || true

echo ""
echo "✅ Agent output saved to: $AGENT_OUTPUT_DIR/"
echo ""

# Show what was generated
echo "📋 Generated files:"
ls -la "$AGENT_OUTPUT_DIR/" 2>/dev/null || echo "   No files generated"
echo ""

echo "📝 Next steps:"
echo "   1. Review the generated files in $AGENT_OUTPUT_DIR/"
echo "   2. Merge them into graal-config/ or use them directly with native-image"
