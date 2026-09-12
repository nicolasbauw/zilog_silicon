//! Sortie audio hôte : convoie les échantillons produits par le PSG vers la
//! carte son via SDL2.
//!
//! Ce module ne contient rien de spécifique au CPC ; il ne s'occupe que du
//! passage du flux émulé vers le matériel réel, avec ce que cela suppose de
//! régulation de latence et de mise en forme du signal.

use bytebox_core::app_log;
use bytebox_core::sound::SAMPLE_RATE;
use sdl2::audio::{AudioQueue, AudioSpecDesired};

/// Taille du bloc demandé à SDL. 512 échantillons à 44,1 kHz font environ
/// 11 ms, soit un bon compromis entre latence et risque de sous-alimentation.
const DEVICE_BUFFER: u16 = 512;

/// Latence maximale tolérée avant de jeter des échantillons. L'émulateur cale
/// ses trames sur une échéance temporelle, mais la moindre dérive entre son
/// horloge et celle de la carte son ferait sinon grossir la file sans fin,
/// jusqu'à un retard audible de plusieurs secondes.
const MAX_LATENCY_SAMPLES: u32 = SAMPLE_RATE / 10;

/// Coussin visé quand la file s'est vidée. L'émulateur ne l'alimente qu'une
/// fois par trame, par salves de 20 ms, alors que la carte son la vide en
/// continu : sans réserve d'avance, la moindre trame un peu longue laisse la
/// carte son à sec, ce qui s'entend comme un craquement.
const TARGET_LATENCY_SAMPLES: u32 = SAMPLE_RATE * 3 / 50; // 3 trames, 60 ms

/// Seuil en dessous duquel on reconstitue le coussin. Au-dessus, on ne touche
/// à rien : rembourrer à chaque trame ne ferait qu'ajouter de la latence.
const MIN_LATENCY_SAMPLES: u32 = SAMPLE_RATE / 50; // 1 trame, 20 ms

/// Décision de régulation pour une trame d'échantillons.
#[derive(Debug, PartialEq, Eq)]
enum Regulation {
    /// Trop de retard accumulé : la trame est jetée.
    Drop,
    /// Constitution du coussin de démarrage. C'est le fonctionnement prévu,
    /// pas un incident : le distinguer évite que le compte rendu ne crie à
    /// chaque lancement, ce qui le rendrait vite illisible.
    Prime { padding: u32 },
    /// Trame envoyée, précédée de `padding` échantillons de silence si le
    /// coussin a dû être reconstitué en cours de route.
    Send { padding: u32 },
}

/// Décide du sort d'une trame en fonction de ce qui reste à jouer.
fn regulate(queued: u32, primed: bool) -> Regulation {
    if queued > MAX_LATENCY_SAMPLES {
        Regulation::Drop
    } else if queued < MIN_LATENCY_SAMPLES {
        let padding = TARGET_LATENCY_SAMPLES - queued;
        if primed {
            Regulation::Send { padding }
        } else {
            Regulation::Prime { padding }
        }
    } else {
        Regulation::Send { padding: 0 }
    }
}

/// En-tête WAV mono 16 bits à la fréquence du PSG.
fn wav_header(samples: u32) -> Vec<u8> {
    let data_len = samples * 2;
    let mut h = Vec::with_capacity(44);
    h.extend(b"RIFF");
    h.extend(&(36 + data_len).to_le_bytes());
    h.extend(b"WAVEfmt ");
    h.extend(&16u32.to_le_bytes());
    h.extend(&1u16.to_le_bytes());
    h.extend(&1u16.to_le_bytes());
    h.extend(&SAMPLE_RATE.to_le_bytes());
    h.extend(&(SAMPLE_RATE * 2).to_le_bytes());
    h.extend(&2u16.to_le_bytes());
    h.extend(&16u16.to_le_bytes());
    h.extend(b"data");
    h.extend(&data_len.to_le_bytes());
    h
}

/// Constante du filtre coupe-continu, pour un pôle vers 20 Hz à 44,1 kHz.
const DC_BLOCKER_R: f32 = 0.9972;

pub struct Audio {
    queue: AudioQueue<f32>,
    volume: f32,
    /// État du filtre coupe-continu (entrée et sortie précédentes).
    last_input: f32,
    last_output: f32,
    /// Tampon de conversion réutilisé d'une trame à l'autre.
    scratch: Vec<f32>,
    /// Bilan de la régulation, exposé au débogueur. Un remplissage insère du
    /// silence dans le flux : la carte son ne saute rien, elle joue ce silence,
    /// et la musique s'en trouve étirée d'autant. Une seule seconde de jeu
    /// avec quelques remplissages suffit à l'entendre traîner.
    refills: u32,
    padded_samples: u64,
    dropped_frames: u32,
    /// Le coussin de démarrage a été constitué.
    primed: bool,
    /// Enregistrement du flux réellement envoyé à la carte son, silence de
    /// remplissage compris. Activé par AMSTRAD_AUDIO_DUMP=<fichier.wav>.
    /// Comparer la durée du fichier au temps réel écoulé mesure directement
    /// tout étirement de la restitution.
    recorder: Option<(std::fs::File, u32)>,
    /// Instant d'ouverture du périphérique, pour rapporter la durée du son
    /// produit au temps réel écoulé.
    opened_at: std::time::Instant,
}

impl Audio {
    /// Ouvre le périphérique audio. Une machine sans carte son utilisable ne
    /// doit pas empêcher l'émulateur de tourner : l'appelant est libre de
    /// continuer sans son en cas d'erreur.
    pub fn new(sdl: &sdl2::Sdl) -> Result<Self, String> {
        let subsystem = sdl.audio()?;
        let desired = AudioSpecDesired {
            freq: Some(SAMPLE_RATE as i32),
            channels: Some(1),
            samples: Some(DEVICE_BUFFER),
        };
        let queue: AudioQueue<f32> = subsystem.open_queue(None, &desired)?;
        queue.resume();

        Ok(Self {
            queue,
            volume: 0.5,
            last_input: 0.0,
            last_output: 0.0,
            scratch: Vec::new(),
            refills: 0,
            padded_samples: 0,
            dropped_frames: 0,
            primed: false,
            opened_at: std::time::Instant::now(),
            recorder: std::env::var("AMSTRAD_AUDIO_DUMP").ok().and_then(|path| {
                match std::fs::File::create(&path) {
                    Ok(mut f) => {
                        // En-tête WAV provisoire, complété à la fermeture.
                        use std::io::Write;
                        let _ = f.write_all(&wav_header(0));
                        app_log!("Recording the audio output to {path}");
                        Some((f, 0))
                    }
                    Err(e) => {
                        app_log!("Can't record the audio output: {e}");
                        None
                    }
                }
            }),
        })
    }

    /// Nombre d'échantillons déjà en attente de lecture par la carte son.
    pub fn queued_samples(&self) -> u32 {
        self.queue.size() / std::mem::size_of::<f32>() as u32
    }

    /// Envoie une trame d'échantillons du PSG vers la carte son.
    pub fn push(&mut self, samples: &[f32]) {
        // Le dernier échantillon est relevé ici, dans la garde même qui
        // écarte les trames vides : garde et usage restent ainsi solidaires,
        // là où un `last().unwrap()` plus bas dépendait, à distance, d'un
        // `is_empty()` qu'un remaniement pouvait déplacer.
        let Some(&last_sample) = samples.last() else {
            return;
        };

        let padding = match regulate(self.queued_samples(), self.primed) {
            // Trop de retard accumulé : on saute cette trame plutôt que de
            // laisser le son dériver durablement derrière l'image. Le filtre
            // est tout de même avancé pour éviter une discontinuité au retour
            // à la normale.
            Regulation::Drop => {
                self.dropped_frames += 1;
                self.last_input = last_sample;
                return;
            }
            Regulation::Prime { padding } => {
                self.primed = true;
                padding
            }
            Regulation::Send { padding } => {
                if padding > 0 {
                    self.refills += 1;
                    self.padded_samples += padding as u64;
                }
                padding
            }
        };

        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.clear();
        scratch.reserve(padding as usize + samples.len());
        scratch.resize(padding as usize, 0.0);
        for &sample in samples {
            let filtered = self.filter(sample);
            scratch.push(filtered);
        }
        let _ = self.queue.queue_audio(&scratch);
        self.record(&scratch);
        self.scratch = scratch;
    }

    /// Ajoute au fichier d'enregistrement, si activé.
    ///
    /// L'en-tête est réécrit à chaque trame : un fichier interrompu en cours
    /// de route (Ctrl+C, plantage) reste ainsi lisible, alors qu'un en-tête
    /// laissé à zéro se relit comme du bruit.
    fn record(&mut self, samples: &[f32]) {
        use std::io::{Seek, SeekFrom, Write};
        if let Some((file, count)) = self.recorder.as_mut() {
            let mut bytes = Vec::with_capacity(samples.len() * 2);
            for &s in samples {
                bytes.extend(&((s.clamp(-1.0, 1.0) * 32000.0) as i16).to_le_bytes());
            }
            let _ = file.seek(SeekFrom::End(0));
            let _ = file.write_all(&bytes);
            *count += samples.len() as u32;
            let header = wav_header(*count);
            let _ = file.seek(SeekFrom::Start(0));
            let _ = file.write_all(&header);
        }
    }

    /// Bilan de la régulation depuis le dernier appel : nombre de
    /// remplissages, silence inséré (en millisecondes), trames jetées.
    pub fn take_stats(&mut self) -> (u32, f32, u32) {
        let stats = (
            self.refills,
            1000.0 * self.padded_samples as f32 / SAMPLE_RATE as f32,
            self.dropped_frames,
        );
        self.refills = 0;
        self.padded_samples = 0;
        self.dropped_frames = 0;
        stats
    }

    /// Volume de sortie, dans [0, 1].
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
    }

    /// Filtre coupe-continu et mise au volume. La sortie du PSG est un signal
    /// positif : sans retrait de la composante continue, chaque note gagnerait
    /// et perdrait un décalage brutal, entendu comme un claquement.
    fn filter(&mut self, sample: f32) -> f32 {
        let output = sample - self.last_input + DC_BLOCKER_R * self.last_output;
        self.last_input = sample;
        self.last_output = output;
        output * self.volume
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        use std::io::{Seek, SeekFrom, Write};
        let elapsed = self.opened_at.elapsed().as_secs_f32();
        if let Some((file, count)) = self.recorder.as_mut() {
            let _ = file.seek(SeekFrom::Start(0));
            let _ = file.write_all(&wav_header(*count));
            let sound = *count as f32 / SAMPLE_RATE as f32;
            let wall = elapsed;
            app_log!(
                "Audio recording closed: {sound:.2} s of sound for {wall:.2} s of real time \
                 (ratio {:.4} - 1.0000 means the sound is not stretched)",
                sound / wall
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nombre d'échantillons que l'émulateur produit pour une trame standard.
    const FRAME: u32 = SAMPLE_RATE / 50;

    #[test]
    fn an_empty_queue_is_refilled_with_a_cushion() {
        // Au démarrage, et après toute sous-alimentation, il faut reconstituer
        // une réserve d'avance : sans elle la carte son retombe à sec dès la
        // trame suivante.
        assert_eq!(
            regulate(0, true),
            Regulation::Send {
                padding: TARGET_LATENCY_SAMPLES
            }
        );
        assert!(
            TARGET_LATENCY_SAMPLES > FRAME,
            "le coussin doit depasser une trame"
        );
    }

    /// Le coussin de démarrage se distingue d'un remplissage subi : le premier
    /// est le fonctionnement normal, le second est le symptôme qu'on cherche à
    /// voir signalé.
    #[test]
    fn the_startup_cushion_is_not_reported_as_an_incident() {
        assert_eq!(
            regulate(0, false),
            Regulation::Prime {
                padding: TARGET_LATENCY_SAMPLES
            }
        );
    }

    #[test]
    fn a_healthy_queue_is_left_alone() {
        // Le régime normal ne doit rien ajouter, sinon la latence grimperait
        // d'une trame à l'autre jusqu'au décrochage entre le son et l'image.
        for queued in [MIN_LATENCY_SAMPLES, 2 * FRAME, MAX_LATENCY_SAMPLES] {
            assert_eq!(regulate(queued, true), Regulation::Send { padding: 0 });
        }
    }

    #[test]
    fn an_overfull_queue_drops_a_frame() {
        assert_eq!(regulate(MAX_LATENCY_SAMPLES + 1, true), Regulation::Drop);
    }

    /// La régulation doit converger : en régime stable, elle ne doit ni
    /// rembourrer ni jeter en boucle.
    #[test]
    fn the_regulation_settles_at_a_stable_latency() {
        let mut queued = 0u32;
        let mut primed = false;
        let mut refills = 0;
        for _ in 0..1000 {
            match regulate(queued, primed) {
                Regulation::Drop => queued -= FRAME,
                Regulation::Prime { padding } => {
                    primed = true;
                    queued += padding + FRAME;
                }
                Regulation::Send { padding } => {
                    if padding > 0 {
                        refills += 1;
                    }
                    queued += padding + FRAME;
                }
            }
            // La carte son consomme une trame par trame émulée.
            queued = queued.saturating_sub(FRAME);
        }
        assert_eq!(refills, 0, "aucun remplissage subi en regime stable");
        assert!(
            (MIN_LATENCY_SAMPLES..=MAX_LATENCY_SAMPLES).contains(&queued),
            "latence stabilisee hors de la plage visee : {queued}"
        );
    }

    /// Le filtre est testé seul : ouvrir un vrai périphérique SDL n'a rien à
    /// faire dans une suite de tests.
    struct DcBlocker {
        last_input: f32,
        last_output: f32,
    }

    impl DcBlocker {
        fn new() -> Self {
            Self {
                last_input: 0.0,
                last_output: 0.0,
            }
        }
        fn filter(&mut self, sample: f32) -> f32 {
            let output = sample - self.last_input + DC_BLOCKER_R * self.last_output;
            self.last_input = sample;
            self.last_output = output;
            output
        }
    }

    #[test]
    fn a_constant_input_decays_to_zero() {
        let mut f = DcBlocker::new();
        let mut last = 0.0;
        for _ in 0..SAMPLE_RATE {
            last = f.filter(0.5);
        }
        assert!(last.abs() < 1e-3, "composante continue restante : {last}");
    }

    #[test]
    fn silence_stays_exactly_silent() {
        let mut f = DcBlocker::new();
        for _ in 0..1000 {
            assert_eq!(f.filter(0.0), 0.0);
        }
    }

    #[test]
    fn a_square_wave_keeps_its_peak_to_peak_amplitude() {
        // Un carré à 1 kHz : le filtre ne doit retirer que le décalage, pas
        // écraser le signal utile.
        let mut f = DcBlocker::new();
        let half = SAMPLE_RATE as usize / 2000;
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for i in 0..SAMPLE_RATE as usize {
            let input = if (i / half) % 2 == 0 { 1.0 } else { 0.0 };
            let out = f.filter(input);
            // On ignore le régime transitoire du début.
            if i > SAMPLE_RATE as usize / 2 {
                min = min.min(out);
                max = max.max(out);
            }
        }
        // Le léger dépassement de 1 est le fléchissement du filtre pendant
        // chaque demi-période, inévitable pour un coupe-continu du premier
        // ordre ; il reste très en deçà de ce qui s'entendrait.
        assert!(
            (0.95..1.05).contains(&(max - min)),
            "amplitude crete a crete {} attendue proche de 1",
            max - min
        );
        assert!(max > 0.0 && min < 0.0, "le signal doit etre recentre");
    }
}
