import * as THREE from "three";

/**
 * Procedural low-poly PS2-style rat (M2 placeholder for the Blender glTF
 * model that arrives in M7). Flat-shaded primitives, rendered at low
 * resolution and upscaled with nearest-neighbor for the period look.
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

function buildRat(): { group: THREE.Group; tail: THREE.Mesh[]; earL: THREE.Mesh; earR: THREE.Mesh; eyes: THREE.Mesh[] } {
  const g = new THREE.Group();

  // body: squashed icosahedron, detail 0 → chunky facets
  const body = new THREE.Mesh(new THREE.IcosahedronGeometry(1.0, 0), flat(FUR));
  body.scale.set(1.25, 0.85, 0.95);
  g.add(body);

  // head: cone snout
  const head = new THREE.Mesh(new THREE.ConeGeometry(0.62, 1.2, 6), flat(FUR));
  head.rotation.z = -Math.PI / 2 - 0.25;
  head.position.set(1.25, 0.32, 0);
  g.add(head);

  // nose
  const nose = new THREE.Mesh(new THREE.IcosahedronGeometry(0.1, 0), flat(PINK));
  nose.position.set(1.86, 0.18, 0);
  g.add(nose);

  // ears: flattened cylinders
  const earGeo = new THREE.CylinderGeometry(0.3, 0.3, 0.08, 6);
  const earL = new THREE.Mesh(earGeo, flat(PINK));
  earL.rotation.x = Math.PI / 2;
  earL.rotation.z = 0.3;
  earL.position.set(0.85, 0.85, 0.35);
  const earR = earL.clone();
  earR.position.z = -0.35;
  earR.rotation.z = -0.3;
  g.add(earL, earR);

  // eyes
  const eyeGeo = new THREE.IcosahedronGeometry(0.09, 0);
  const eyeL = new THREE.Mesh(eyeGeo, flat(EYE));
  eyeL.position.set(1.28, 0.55, 0.3);
  const eyeR = eyeL.clone();
  eyeR.position.z = -0.3;
  g.add(eyeL, eyeR);

  // feet
  const footGeo = new THREE.BoxGeometry(0.34, 0.18, 0.22);
  for (const [x, z] of [
    [0.6, 0.45],
    [0.6, -0.45],
    [-0.6, 0.5],
    [-0.6, -0.5],
  ]) {
    const foot = new THREE.Mesh(footGeo, flat(FUR_DARK));
    foot.position.set(x, -0.78, z);
    g.add(foot);
  }

  // tail: chain of thin boxes curving away
  const tail: THREE.Mesh[] = [];
  let prev = new THREE.Vector3(-1.15, -0.2, 0);
  for (let i = 0; i < 6; i++) {
    const seg = new THREE.Mesh(
      new THREE.BoxGeometry(0.42, 0.09 - i * 0.008, 0.09 - i * 0.008),
      flat(PINK),
    );
    seg.position.copy(prev).add(new THREE.Vector3(-0.36, 0.02 * i, 0));
    prev = seg.position.clone();
    g.add(seg);
    tail.push(seg);
  }

  return { group: g, tail, earL, earR, eyes: [eyeL, eyeR] };
}

export function mountRat(canvas: HTMLCanvasElement): Rat3D {
  const RES = 240; // render low, upscale nearest → PS2 chunk
  const renderer = new THREE.WebGLRenderer({ canvas, alpha: true, antialias: false });
  renderer.setSize(RES, RES, false);
  renderer.setPixelRatio(1);

  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(38, 1, 0.1, 50);
  camera.position.set(0.4, 1.6, 5.2);
  camera.lookAt(0.2, 0, 0);

  scene.add(new THREE.AmbientLight(0xb0bcd0, 1.1));
  const key = new THREE.DirectionalLight(0xfff4dd, 1.6);
  key.position.set(2, 4, 3);
  scene.add(key);

  const { group, tail, earL, earR, eyes } = buildRat();
  group.position.y = -0.1;
  group.rotation.y = -0.4;
  scene.add(group);

  let mode: "active" | "away" = "active";
  let disposed = false;
  let blinkUntil = 0;
  let nextBlink = performance.now() + 2500;

  function animate(now: number) {
    if (disposed) return;
    requestAnimationFrame(animate);
    const t = now / 1000;

    if (mode === "active") {
      // idle: bob, sniff, tail sway — quantized to 12 fps for the retro feel
      const qt = Math.floor(t * 12) / 12;
      group.position.y = -0.1 + Math.sin(qt * 2.2) * 0.045;
      group.rotation.z = Math.sin(qt * 2.2 + 1) * 0.02;
      group.rotation.x = 0;
      tail.forEach((seg, i) => {
        seg.position.z = Math.sin(qt * 1.7 + i * 0.7) * 0.16 * (i / tail.length);
      });
      earL.rotation.y = Math.sin(qt * 0.9) * 0.15;
      earR.rotation.y = -Math.sin(qt * 0.9 + 0.4) * 0.15;
    } else {
      // away: lie down, slow breathing
      group.rotation.x = Math.PI / 14;
      group.position.y = -0.45 + Math.sin(t * 0.8) * 0.015;
      group.rotation.z = 0.35;
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
