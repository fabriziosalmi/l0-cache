#!/bin/bash
# benchmark.sh - Simulate a noisy tool (e.g. docker logs or test runner)

# Create a mock noisy command
cat << 'EOF' > mock_tool.sh
#!/bin/bash
# 1. Output 5000 lines of identical successful logs with timestamps
for i in {1..5000}; do
    echo "2024-10-12T10:30:00.123Z [INFO] Successfully processed item $i but the prefix is the same"
done

# 2. Output 100 lines of unique stack traces/errors
for i in {1..100}; do
    echo "2024-10-12T10:30:01.000Z [ERROR] NullPointerException at module.js:$i"
done

# 3. Exit with error to simulate a failed test/build
exit 1
EOF
chmod +x mock_tool.sh

echo "========================================"
echo "📊 BASELINE (Nativo, senza l0-cache)"
echo "========================================"
./mock_tool.sh > out_native.txt 2>&1
echo "Linee: $(wc -l < out_native.txt)"
echo "Dimensione: $(ls -lh out_native.txt | awk '{print $5}')"
echo ""

echo "========================================"
echo "🛡️  L0-CACHE (No Auto - Default 30 head/tail)"
echo "========================================"
time ./target/release/l0-cache --no-auto ./mock_tool.sh > out_no_auto.txt 2>&1
echo "Linee: $(wc -l < out_no_auto.txt)"
echo "Dimensione: $(ls -lh out_no_auto.txt | awk '{print $5}')"
echo ""

echo "========================================"
echo "🤖 L0-CACHE (Auto-Tuning attivato)"
echo "========================================"
# Clean metrics to ensure a fresh auto-tuning state
export XDG_DATA_HOME="/tmp/l0-cache-bench"
rm -rf "$XDG_DATA_HOME"

echo "Esecuzione 1 (Fallimento 1)..."
./target/release/l0-cache --auto ./mock_tool.sh > /dev/null 2>&1
echo "Esecuzione 2 (Fallimento 2)..."
./target/release/l0-cache --auto ./mock_tool.sh > /dev/null 2>&1
echo "Esecuzione 3 (Fallimento 3 - coda estesa)..."
time ./target/release/l0-cache --auto ./mock_tool.sh > out_auto.txt 2>&1
echo "Linee: $(wc -l < out_auto.txt)"
echo "Dimensione: $(ls -lh out_auto.txt | awk '{print $5}')"
echo ""

# Cleanup
rm mock_tool.sh out_native.txt out_no_auto.txt out_auto.txt
