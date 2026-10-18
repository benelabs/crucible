#!/usr/bin/env node

/**
 * Bundle Size Validation Script
 * Validates that the production bundle meets performance requirements
 * - Initial JS < 150KB (gzipped)
 * - Total bundle < 600KB (gzipped)
 * - Individual chunks < 300KB (gzipped)
 */

const fs = require('fs');
const path = require('path');
const zlib = require('zlib');
const { promisify } = require('util');

const gzip = promisify(zlib.gzip);

const LIMITS = {
  INITIAL_JS_KB: 150,
  TOTAL_BUNDLE_KB: 600,
  CHUNK_KB: 300,
};

const DIST_DIR = path.join(__dirname, '../dist');
const JS_DIR = path.join(DIST_DIR, 'js');

async function getFileSize(filePath) {
  const content = fs.readFileSync(filePath);
  const gzipped = await gzip(content);
  return {
    raw: content.length,
    gzipped: gzipped.length,
  };
}

async function validateBundleSize() {
  console.log('📦 Validating Bundle Size...\n');

  if (!fs.existsSync(DIST_DIR)) {
    console.error('❌ dist directory not found. Run `npm run build` first.');
    process.exit(1);
  }

  if (!fs.existsSync(JS_DIR)) {
    console.error('❌ dist/js directory not found.');
    process.exit(1);
  }

  const files = fs.readdirSync(JS_DIR).filter(f => f.endsWith('.js'));
  
  if (files.length === 0) {
    console.error('❌ No JavaScript files found in dist/js');
    process.exit(1);
  }

  const results = [];
  let totalSize = 0;
  let initialSize = 0;

  console.log('File Analysis:\n');
  console.log('File Name'.padEnd(40) + 'Raw'.padEnd(15) + 'Gzipped'.padEnd(15) + 'Status');
  console.log('─'.repeat(85));

  for (const file of files) {
    const filePath = path.join(JS_DIR, file);
    const sizes = await getFileSize(filePath);
    const sizeKB = sizes.gzipped / 1024;
    totalSize += sizes.gzipped;

    // Determine if this is an entry point (main chunk)
    const isInitial = file.includes('main') || file.includes('index');
    if (isInitial) {
      initialSize += sizes.gzipped;
    }

    const status = sizeKB > LIMITS.CHUNK_KB ? '⚠️  WARN' : '✅ OK';
    
    results.push({
      file,
      isInitial,
      sizeKB,
      rawBytes: sizes.raw,
      gzippedBytes: sizes.gzipped,
      status: sizeKB > LIMITS.CHUNK_KB ? 'warn' : 'ok',
    });

    console.log(
      file.substring(0, 39).padEnd(40) +
      `${(sizes.raw / 1024).toFixed(2)} KB`.padEnd(15) +
      `${sizeKB.toFixed(2)} KB`.padEnd(15) +
      status
    );
  }

  const initialSizeKB = initialSize / 1024;
  const totalSizeKB = totalSize / 1024;

  console.log('─'.repeat(85));
  console.log('\nSummary:\n');

  const checks = [
    {
      name: 'Initial JS Bundle',
      value: initialSizeKB,
      limit: LIMITS.INITIAL_JS_KB,
      unit: 'KB',
      critical: true,
    },
    {
      name: 'Total Bundle Size',
      value: totalSizeKB,
      limit: LIMITS.TOTAL_BUNDLE_KB,
      unit: 'KB',
      critical: false,
    },
  ];

  let hasErrors = false;

  for (const check of checks) {
    const status = check.value <= check.limit ? '✅' : check.critical ? '❌' : '⚠️';
    const percentage = ((check.value / check.limit) * 100).toFixed(1);
    console.log(
      `${status} ${check.name.padEnd(30)} ${check.value.toFixed(2)} ${check.unit} / ${check.limit} ${check.unit} (${percentage}%)`
    );

    if (check.value > check.limit && check.critical) {
      hasErrors = true;
    }
  }

  // Check individual chunks
  console.log('\nChunk Analysis:');
  const oversizedChunks = results.filter(r => r.sizeKB > LIMITS.CHUNK_KB);
  
  if (oversizedChunks.length > 0) {
    console.log('\n⚠️  Oversized Chunks (> ' + LIMITS.CHUNK_KB + ' KB):');
    for (const chunk of oversizedChunks) {
      const diff = (chunk.sizeKB - LIMITS.CHUNK_KB).toFixed(2);
      console.log(`   ${chunk.file.padEnd(30)} ${chunk.sizeKB.toFixed(2)} KB (${diff} KB over limit)`);
    }
  } else {
    console.log('\n✅ All chunks within size limits');
  }

  // Performance metrics
  console.log('\nPerformance Estimates:\n');
  const estimatedLoadTime3G = (totalSize / (1024 * 1024 * 0.400)) * 1000; // 400 Kbps
  const estimatedLoadTime4G = (totalSize / (1024 * 1024 * 4)) * 1000;     // 4 Mbps

  console.log(`   3G Network (400 Kbps): ~${estimatedLoadTime3G.toFixed(0)}ms`);
  console.log(`   4G Network (4 Mbps):   ~${estimatedLoadTime4G.toFixed(0)}ms`);

  // Report
  console.log('\n' + '═'.repeat(85));
  
  if (hasErrors) {
    console.error('\n❌ BUNDLE SIZE VALIDATION FAILED\n');
    console.error('The following limits were exceeded:');
    if (initialSizeKB > LIMITS.INITIAL_JS_KB) {
      console.error(`   - Initial JS: ${initialSizeKB.toFixed(2)} KB > ${LIMITS.INITIAL_JS_KB} KB`);
    }
    if (totalSizeKB > LIMITS.TOTAL_BUNDLE_KB) {
      console.error(`   - Total bundle: ${totalSizeKB.toFixed(2)} KB > ${LIMITS.TOTAL_BUNDLE_KB} KB`);
    }
    process.exit(1);
  } else {
    console.log('\n✅ Bundle size validation passed!\n');
    
    // Write metrics for CI
    const metrics = {
      timestamp: new Date().toISOString(),
      initialJS_KB: parseFloat(initialSizeKB.toFixed(2)),
      totalBundle_KB: parseFloat(totalSizeKB.toFixed(2)),
      chunks: results.map(r => ({
        name: r.file,
        size_KB: parseFloat(r.sizeKB.toFixed(2)),
        status: r.status,
      })),
    };

    fs.writeFileSync(
      path.join(DIST_DIR, 'bundle-metrics.json'),
      JSON.stringify(metrics, null, 2)
    );

    console.log('📊 Metrics saved to dist/bundle-metrics.json\n');
  }
}

validateBundleSize().catch(err => {
  console.error('Error during bundle validation:', err);
  process.exit(1);
});
