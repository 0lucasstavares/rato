import * as THREE from "three";

/**
 * Procedural low-poly PS2-style rat (M2 placeholder for the Blender glTF
 * model that arrives in M7). Biped bust, framed torso-up: the canvas bottom
 * edge crops him at the waist, so he rises out of the screen bottom.
 * Flat-shaded primitives, rendered low-res and upscaled nearest-neighbor.
 */
export interface Rat3D {
  setMode(mode: "active" | "away"): void;
  dispose(): void;
}

const FUR = 0x8d93a1;
const FUR_DARK = 0x6c7280;
const PINK = 0xd99aa8;
const EYE = 0x16181d;

function flat(color: number): THREE.MeshLambertMaterial {
  return new THREE.MeshLambertMaterial({ color, flatShading: true });
}

interface RatParts {
  group: THREE.Group;
  head: THREE.Group;
  earL: THREE.Mesh;
  earR: THREE.Mesh;
  eyes: THREE.Mesh[];
  armL: THREE.Group;
  armR: THREE.Group;
}

/** One arm: pivot at the shoulder, cylinder hanging down, mitt hand at the end. */
function buildArm(side: 1 | -1): THREE.Group {
  const arm = new THREE.Group();
  const upper = new THREE.Mesh(new THREE.CylinderGeometry(0.13, 0.11, 0.8, 5), flat(FUR_DARK));
  upper.position.y = -0.4;
  arm.add(upper);

  const hand = new THREE.Group();
  const palm = new THREE.Mesh(new THREE.IcosahedronGeometry(0.16, 0), flat(PINK));
  palm.scale.set(1, 1.15, 0.8);
  hand.add(palm);
  const thumb = new THREE.Mesh(new THREE.BoxGeometry(0.07, 0.16, 0.07), flat(PINK));
  thumb.position.set(side * -0.14, 0.02, 0.06);
  thumb.rotation.z = side * 0.5;
  hand.add(thumb);
  hand.position.y = -0.88;
  arm.add(hand);
  return arm;
}

function buildRat(): RatParts {
  const g = new THREE.Group();

  // torso: plump pear — hips sphere (partially below the crop line) + chest
  const hips = new THREE.Mesh(new THREE.IcosahedronGeometry(1.0, 0), flat(FUR));
  hips.scale.set(1.15, 1.0, 0.85);
  hips.position.y = -1.15;
  g.add(hips);

  const chest = new THREE.Mesh(new THREE.IcosahedronGeometry(0.75, 0), flat(FUR));
  chest.scale.set(0.95, 0.9, 0.78);
  chest.position.y = -0.3;
  g.add(chest);

  // belly patch
  const belly = new THREE.Mesh(new THREE.IcosahedronGeometry(0.62, 0), flat(0xb9bec9));
  belly.scale.set(0.72, 0.85, 0.5);
  belly.position.set(0, -0.85, 0.55);
  g.add(belly);

  // head group (head + snout + nose + ears + eyes) — pivots for the away slump
  const head = new THREE.Group();
  head.position.y = 0.42; // neck joint

  const skull = new THREE.Mesh(new THREE.IcosahedronGeometry(0.62, 1), flat(FUR));
  skull.position.y = 0.3;
  head.add(skull);

  // snout: blunt cone pointing at the camera
  const snout = new THREE.Mesh(new THREE.ConeGeometry(0.3, 0.5, 6), flat(FUR));
  snout.rotation.x = Math.PI / 2;
  snout.position.set(0, 0.16, 0.62);
  head.add(snout);

  const nose = new THREE.Mesh(new THREE.IcosahedronGeometry(0.09, 0), flat(PINK));
  nose.position.set(0, 0.16, 0.9);
  head.add(nose);

  // ears: big discs facing the camera, pink inner / grey rim
  const earRim = new THREE.CylinderGeometry(0.36, 0.36, 0.07, 8);
  const earInner = new THREE.CylinderGeometry(0.26, 0.26, 0.08, 8);
  const earL = new THREE.Mesh(earRim, flat(FUR_DARK));
  earL.rotation.x = Math.PI / 2;
  earL.rotation.z = 0.22;
  earL.position.set(-0.52, 0.85, -0.05);
  const innerL = new THREE.Mesh(earInner, flat(PINK));
  innerL.position.y = 0.01; // proud of the rim, toward the camera after rotation
  earL.add(innerL);
  const earR = earL.clone();
  earR.position.x = 0.52;
  earR.rotation.z = -0.22;
  head.add(earL, earR);

  // eyes: black beads, front of the skull
  const eyeGeo = new THREE.IcosahedronGeometry(0.09, 0);
  const eyeL = new THREE.Mesh(eyeGeo, flat(EYE));
  eyeL.position.set(-0.24, 0.42, 0.5);
  const eyeR = eyeL.clone();
  eyeR.position.x = 0.24;
  head.add(eyeL, eyeR);

  g.add(head);

  // arms at the shoulders, hanging relaxed
  const armL = buildArm(-1);
  armL.position.set(-0.72, 0.05, 0.05);
  armL.rotation.z = -0.18;
  const armR = buildArm(1);
  armR.position.set(0.72, 0.05, 0.05);
  armR.rotation.z = 0.18;
  g.add(armL, armR);

  return { group: g, head, earL, earR, eyes: [eyeL, eyeR], armL, armR };
}

export function mountRat(canvas: HTMLCanvasElement): Rat3D {
  // render low, upscale nearest → PS2 chunk; 5:6 matches the 180×216 css box
  const RES_W = 150;
  const RES_H = 180;
  const renderer = new THREE.WebGLRenderer({ canvas, alpha: true, antialias: false });
  renderer.setSize(RES_W, RES_H, false);
  renderer.setPixelRatio(1);

  const scene = new THREE.Scene();
  // visible height ≈ 3.25 world units at z=0; crop line at y≈-1.6
  const camera = new THREE.PerspectiveCamera(36, RES_W / RES_H, 0.1, 50);
  camera.position.set(0, 0.05, 5.0);
  camera.lookAt(0, 0.05, 0);

  scene.add(new THREE.AmbientLight(0xb0bcd0, 1.1));
  const key = new THREE.DirectionalLight(0xfff4dd, 1.6);
  key.position.set(2, 4, 3);
  scene.add(key);

  const { group, head, earL, earR, eyes, armL, armR } = buildRat();
  scene.add(group);

  let mode: "active" | "away" = "active";
  let disposed = false;
  let blinkUntil = 0;
  let nextBlink = performance.now() + 2500;
  // rare idle flourish: raise the right hand in a small wave
  let waveStart = -1;
  let nextWave = performance.now() + 15000 + Math.random() * 25000;
  const WAVE_MS = 2200;
  const ARM_REST = 0.18;

  function animate(now: number) {
    if (disposed) return;
    requestAnimationFrame(animate);
    const t = now / 1000;
    // idle motion quantized to 12 fps for the retro feel
    const qt = Math.floor(t * 12) / 12;

    if (mode === "active") {
      // breathing bob + slight sway
      group.position.y = Math.sin(qt * 2.0) * 0.03;
      group.rotation.z = Math.sin(qt * 2.0 + 1) * 0.012;
      head.rotation.x = 0;
      head.rotation.z = Math.sin(qt * 0.7) * 0.03;
      earL.rotation.y = Math.sin(qt * 0.9) * 0.12;
      earR.rotation.y = -Math.sin(qt * 0.9 + 0.4) * 0.12;
      armL.rotation.z = -ARM_REST + Math.sin(qt * 2.0 + 0.5) * 0.02;

      // wave flourish
      if (waveStart < 0 && now > nextWave) waveStart = now;
      if (waveStart >= 0) {
        const p = (now - waveStart) / WAVE_MS;
        if (p >= 1) {
          waveStart = -1;
          nextWave = now + 20000 + Math.random() * 20000;
          armR.rotation.z = ARM_REST;
        } else {
          // raise (0–0.25), wiggle (0.25–0.75), lower (0.75–1) — stepped
          const raise = p < 0.25 ? p / 0.25 : p > 0.75 ? (1 - p) / 0.25 : 1;
          const qRaise = Math.floor(raise * 6) / 6;
          const wiggle = p >= 0.25 && p <= 0.75 ? Math.sin(qt * 14) * 0.25 : 0;
          armR.rotation.z = ARM_REST + qRaise * 2.1 + wiggle;
        }
      } else {
        armR.rotation.z = ARM_REST + Math.sin(qt * 2.0) * 0.02;
      }
    } else {
      // away: slump — head down, ears drooped, arms dangling, slow breathing
      group.position.y = Math.sin(t * 0.8) * 0.012 - 0.06;
      group.rotation.z = 0;
      head.rotation.x = 0.38;
      head.rotation.z = 0.06;
      earL.rotation.y = 0.45;
      earR.rotation.y = -0.45;
      armL.rotation.z = -0.32;
      armR.rotation.z = 0.32;
    }

    // blink
    if (now > nextBlink) {
      blinkUntil = now + 130;
      nextBlink = now + 1800 + Math.random() * 3200;
    }
    const blinking = now < blinkUntil || mode === "away";
    eyes.forEach((e) => e.scale.setY(blinking ? 0.12 : 1));

    renderer.render(scene, camera);
  }
  requestAnimationFrame(animate);

  return {
    setMode(m) {
      mode = m;
    },
    dispose() {
      disposed = true;
      renderer.dispose();
    },
  };
}
