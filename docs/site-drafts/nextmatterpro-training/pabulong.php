<?php
/*
 * DRAFT. Do not upload until a Developer ID signed and notarized installer exists.
 * Upload to /training/ next to pabulong-terms.php, together with:
 *   assets/img/pabulong/pabulong-horizontal-reversed.svg
 *   downloads/.htaccess
 *   downloads/Pabulong-<version>-arm64.pkg  (from outputs/offline-full/.../)
 * One release at a time: copy version, size and SHA-256 from the release.json
 * that Scripts/release.sh writes next to the .pkg. The values below come from the
 * Apple Development test build.
 */
require_once __DIR__ . '/includes/functions.php';

$pabulongVersion = '0.1.0';
$pabulongFile = 'Pabulong-' . $pabulongVersion . '-arm64.pkg';
$pabulongSize = '6.9 MB';
$pabulongSha256 = '8aa0e7040dd83c0f5ca884d9255a8369e47ccc78ca982019117fc6d6cd8ba57a';
$pabulongDownload = 'downloads/' . $pabulongFile;
$pabulongSource = 'https://github.com/maespela/pabulong';
$pabulongUninstall = 'sudo /Applications/Pabulong.app/Contents/Resources/uninstall.sh';

$pageTitle = APP_NAME . ' | Pabulong: Free Noise-Cancelling Microphone for Mac and Windows';
include __DIR__ . '/includes/layout-top.php';
?>
<style>
  .pbl-hero { padding: 56px 0 48px; }
  .pbl-hero-grid { display: grid; gap: 32px; align-items: center; }
  @media (min-width: 960px) { .pbl-hero-grid { grid-template-columns: minmax(0, 1fr) minmax(0, 440px); } }
  .pbl-hero h1 { margin: 0 0 16px; }
  .pbl-mark {
    display: grid;
    place-items: center;
    aspect-ratio: 16 / 9;
    padding: 32px;
    border-radius: 20px;
    background: radial-gradient(ellipse at center, #123B6D 0%, #0B1F3A 75%);
  }
  .pbl-mark img { display: block; width: min(100%, 320px); height: auto; }
  .pbl-cta { display: flex; flex-wrap: wrap; gap: 12px; margin: 24px 0 12px; }
  .pbl-soon { cursor: default; opacity: 0.6; pointer-events: none; }
  .pbl-download { display: flex; flex-direction: column; gap: 4px; }
  .pbl-download .btn { align-self: flex-start; margin-top: auto; }
  .pbl-download p { margin: 0 0 8px; }
  .pbl-small { font-size: 0.92rem; color: var(--nmp-muted, #5B6B7C); margin: 6px 0; }
  .pbl-page a:not(.btn) { color: #1A3CFF; text-decoration: underline; text-underline-offset: 2px; }
  .pbl-page h2 { margin: 0 0 16px; }
  .pbl-page h3 { margin: 24px 0 8px; }
  .pbl-narrow > * { max-width: 760px; }
  .pbl-features { display: grid; gap: 20px; margin: 24px 0; }
  @media (min-width: 760px) { .pbl-features { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  .pbl-features .panel { padding: 24px; }
  .pbl-features h3, .pbl-page .panel h3 { margin: 0 0 8px; }
  .pbl-features p { margin: 0; }
  .pbl-steps li + li { margin-top: 12px; }
  .pbl-code {
    background: #F7F7F8;
    border: 1px solid var(--nmp-border, rgba(18,59,109,0.12));
    border-radius: 10px;
    padding: 14px 16px;
    font-size: 0.85rem;
    overflow-x: auto;
  }
  .pbl-code code, .pbl-hash { overflow-wrap: anywhere; }
  .pbl-details summary { cursor: pointer; font-weight: 600; }
</style>

<main id="main" class="pbl-page">
  <section class="pbl-hero">
    <div class="container pbl-hero-grid">
      <div>
        <h1>Pabulong cleans up your microphone on calls</h1>
        <p class="lead">Pabulong adds a microphone called Pabulong Mic to your computer. Choose it in your call app, and the people on the call hear your voice without the noise around you.</p>
        <p>It’s free, it needs no account, and your audio never leaves your computer.</p>
        <div class="pbl-cta">
          <a class="btn btn-primary" href="<?= e($pabulongDownload) ?>" download>Download for Mac</a>
          <span class="btn btn-outline pbl-soon" aria-disabled="true">Windows: coming soon</span>
        </div>
        <p class="pbl-small">Version <?= e($pabulongVersion) ?>, <?= e($pabulongSize) ?>. Needs a Mac with Apple silicon and macOS 14 or later. <a href="#install">How to install</a></p>
        <p class="pbl-small">By downloading Pabulong you agree to the <a href="pabulong-terms.php">Terms of Use</a>.</p>
      </div>
      <div class="pbl-mark">
        <img src="assets/img/pabulong/pabulong-horizontal-reversed.svg" width="1131" height="200" alt="Pabulong">
      </div>
    </div>
  </section>

  <section class="section section-soft" aria-labelledby="removes">
    <div class="container">
      <h2 id="removes">What it removes</h2>
      <div class="pbl-features">
        <article class="panel">
          <h3>Background noise</h3>
          <p>Fans, traffic, rain on the roof and the TV in the next room. A noise filter trained on speech removes them and keeps your voice.</p>
        </article>
        <article class="panel">
          <h3>Echo from your speakers</h3>
          <p>On calls without headphones, the other person can hear themselves come back through your microphone. Pabulong removes what your speakers play before it reaches the call.</p>
        </article>
        <article class="panel">
          <h3>Other people talking near you</h3>
          <p>Family in the same room or a coworker at the next desk. Pabulong turns down voices that are not yours. Choose Light, Balanced or Strong.</p>
        </article>
        <article class="panel">
          <h3>Call recording, if you want it</h3>
          <p>Pabulong can save your calls as audio files on your computer. It is off until you turn it on, and it reminds you to get everyone’s consent first.</p>
        </article>
      </div>
      <p style="max-width: 760px;">Recording a call without everyone’s consent is illegal in many places, including the Philippines and several U.S. states. If you record, you are responsible for following the laws where you and everyone on the call are. <a href="pabulong-terms.php#recording">Read section 5 of the Terms</a>.</p>
    </div>
  </section>

  <section class="section" aria-labelledby="private">
    <div class="container pbl-narrow">
      <h2 id="private">No account, no subscription, nothing sent anywhere</h2>
      <p>Pabulong processes your voice on your computer and makes no internet connections. There is no account to create, no subscription to pay and no server that hears your calls.</p>
      <p>The code is open source under the MIT License. You or anyone you trust can read it on <a href="<?= e($pabulongSource) ?>" target="_blank" rel="noopener">GitHub</a>.</p>
      <p>The <a href="pabulong-privacy.php">Pabulong Privacy Notice</a> and the <a href="pabulong-terms.php">Pabulong Terms of Use</a> have the details.</p>
    </div>
  </section>

  <section class="section section-soft" aria-labelledby="download">
    <div class="container">
      <h2 id="download">Download Pabulong</h2>
      <div class="pbl-features">
        <article class="panel pbl-download">
          <h3>Mac with Apple silicon</h3>
          <p>Version <?= e($pabulongVersion) ?> for macOS 14 Sonoma or later, on Macs with an M1 chip or newer.</p>
          <p class="pbl-small"><?= e($pabulongFile) ?>, <?= e($pabulongSize) ?></p>
          <a class="btn btn-primary" href="<?= e($pabulongDownload) ?>" download>Download for Mac</a>
        </article>
        <article class="panel pbl-download">
          <h3>Windows</h3>
          <p>The Windows version is coming soon. It will be free too.</p>
          <span class="btn btn-outline pbl-soon" aria-disabled="true">Coming soon</span>
        </article>
      </div>
      <p class="pbl-small">By downloading Pabulong you agree to the <a href="pabulong-terms.php">Terms of Use</a>. Not sure which chip your Mac has? Open the Apple menu and choose About This Mac.</p>
      <details class="pbl-details">
        <summary>Check that your download is the real one</summary>
        <p>If the SHA-256 checksum of your file matches the one below, it is exactly the file we published. Open Terminal and run <code>shasum -a 256 ~/Downloads/<?= e($pabulongFile) ?></code>.</p>
        <pre class="pbl-code"><code><?= e($pabulongSha256 . '  ' . $pabulongFile) ?></code></pre>
        <p>Only download Pabulong from this page, from <a href="https://vastarterhub.com/pabulong/">vastarterhub.com</a> or from our <a href="<?= e($pabulongSource) ?>" target="_blank" rel="noopener">GitHub page</a>. It is free, so anyone who asks you to pay for it did not get it from us.</p>
      </details>
    </div>
  </section>

  <section class="section" id="install" aria-labelledby="install-title">
    <div class="container pbl-narrow">
      <h2 id="install-title">Installing it on a Mac</h2>
      <ol class="pbl-steps">
        <li>Open <strong><?= e($pabulongFile) ?></strong> from your Downloads folder.</li>
        <li>Click Continue, agree to the license, then click Install.</li>
        <li>Enter your Mac’s password when asked. The installer needs it to add the Pabulong Mic audio driver. Your Mac’s sound stops for a second while the driver loads.</li>
        <li>Open Pabulong from your Applications folder. Its icon appears in the menu bar at the top of the screen.</li>
        <li>Accept the Terms of Use. When macOS asks, allow microphone access, and allow Pabulong to hear your speakers. It uses your speakers’ sound only to remove echo.</li>
        <li>In your call app’s audio settings, choose Pabulong Mic as the microphone. Leave the speaker setting as it is.</li>
      </ol>
      <h3>Uninstalling</h3>
      <p>Open Terminal, run this command and enter your Mac’s password:</p>
      <pre class="pbl-code"><code><?= e($pabulongUninstall) ?></code></pre>
      <p>Your settings and recordings stay on your Mac. To delete them too, add <code>--delete-data</code> to the end of the command.</p>
    </div>
  </section>

  <section class="section section-soft" aria-labelledby="questions">
    <div class="container pbl-narrow">
      <h2 id="questions">Questions</h2>
      <h3>Which call apps does it work with?</h3>
      <p>Any app that lets you choose a microphone. Choose Pabulong Mic in that app’s audio settings.</p>
      <h3>When is the Windows version coming?</h3>
      <p>Soon. It is in the works and will be free too. Until then, Pabulong runs on Macs with Apple silicon (M1 or later) and macOS 14 Sonoma or later.</p>
      <h3>Does it work on an Intel Mac?</h3>
      <p>No. The Mac version needs Apple silicon (M1 or later).</p>
      <h3>Does it upload my calls?</h3>
      <p>No. Pabulong has no account and makes no internet connections. Your microphone audio, your speaker audio and any recordings stay on your computer.</p>
    </div>
  </section>

  <section class="section" aria-labelledby="maker">
    <div class="container pbl-narrow">
      <h2 id="maker">Who makes it</h2>
      <p>Pabulong is made by Next Matter Pro. Found a bug or have an idea? <a href="<?= e($pabulongSource) ?>/issues" target="_blank" rel="noopener">Open an issue on GitHub</a> or email <a href="mailto:clientcare@nextmatterpro.com">clientcare@nextmatterpro.com</a>.</p>
    </div>
  </section>
</main>
<?php include __DIR__ . '/includes/layout-bottom.php'; ?>
