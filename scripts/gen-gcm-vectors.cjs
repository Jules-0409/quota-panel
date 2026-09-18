// 一次性脚本：用 Node 的 OpenSSL 实现生成 AES-256-GCM 测试向量。
// 目的：Droid CLI 是 Node 写的，IV 是 16 字节，正是 credentials/gcm.rs 手搓 GCM 的原因。
// 拿 Node 生成的密文去喂 Rust 解密，才能证明手搓实现和真实写出方一致。
// 用完即弃，不要提交。
const crypto = require('crypto');

const key = Buffer.from(
  '000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f',
  'hex'
);
const pt = '{"access_token":"test-token-abc123","active_organization_id":"org_test_42"}';

function enc(ivHex, plain) {
  const iv = Buffer.from(ivHex, 'hex');
  const c = crypto.createCipheriv('aes-256-gcm', key, iv);
  const ct = Buffer.concat([c.update(plain, 'utf8'), c.final()]);
  return {
    iv: iv.toString('base64'),
    tag: c.getAuthTag().toString('base64'),
    ct: ct.toString('base64'),
  };
}

function emit(name, v) {
  console.log(name + '_IV=' + v.iv);
  console.log(name + '_TAG=' + v.tag);
  console.log(name + '_CT=' + v.ct);
}

emit('IV16', enc('0102030405060708090a0b0c0d0e0f10', pt));
emit('IV12', enc('0102030405060708090a0b0c', pt));

try {
  emit('IV32', enc('0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20', pt));
} catch (e) {
  console.log('IV32=UNSUPPORTED: ' + e.message);
}

// 空明文：边界，GCM 只有 tag 没有密文
emit('EMPTY', enc('0102030405060708090a0b0c0d0e0f10', ''));
