//! Windows: следит за папками программ Kill Switch. Фильтр брандмауэра стоит на конкретный exe, а программы
//! обновляются в новые папки (Roblox `Versions\version-…`, Discord `app-…`): новый exe надо закрыть сразу,
//! а не при обслуживании раз в 10 минут — иначе обновлённая программа успевает выйти в сеть напрямую.

use crate::engine::Msg;
use notify::event::ModifyKind;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use tokio::sync::mpsc;

pub struct KsWatch {
    watcher: RecommendedWatcher,
    /// Папки, о которых просили (и те, что пока не нашлись): сравниваем с ними, чтобы не пересоздавать слежку.
    wanted: Vec<String>,
    watching: Vec<String>,
}

impl KsWatch {
    pub fn new(tx: mpsc::Sender<Msg>) -> notify::Result<Self> {
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            // Появился файл или папка, переименовали: новая версия программы. Запись в уже известные
            // файлы (игра качает обновление, пишет логи) пропускаем — путей exe она не меняет.
            let fresh = matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(_)));
            let relevant = ev.paths.iter().any(|p| is_exe(p) || p.is_dir());
            if fresh && relevant {
                // Очередь полна — проверка и так впереди.
                let _ = tx.try_send(Msg::KsFolderChanged);
            }
        })?;
        Ok(Self { watcher, wanted: Vec::new(), watching: Vec::new() })
    }

    /// Следить ровно за этими папками.
    pub fn set(&mut self, folders: Vec<String>) {
        if self.wanted == folders {
            return;
        }
        for f in self.watching.drain(..) {
            let _ = self.watcher.unwatch(Path::new(&f));
        }
        for f in &folders {
            match self.watcher.watch(Path::new(f), RecursiveMode::Recursive) {
                Ok(()) => self.watching.push(f.clone()),
                Err(e) => tracing::warn!("Kill Switch: за папкой программы не следить: {e}"),
            }
        }
        tracing::info!("Kill Switch: слежу за папками программ: {} из {}", self.watching.len(), folders.len());
        self.wanted = folders;
    }
}

fn is_exe(p: &Path) -> bool {
    p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Пришло ли сообщение о папке за `wait`; очередь после этого пуста.
    fn got(rx: &mut mpsc::Receiver<Msg>, wait: Duration) -> bool {
        let until = Instant::now() + wait;
        let mut seen = false;
        while Instant::now() < until {
            while let Ok(m) = rx.try_recv() {
                seen |= matches!(m, Msg::KsFolderChanged);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        seen
    }

    #[test]
    fn new_version_folder_is_noticed_log_writes_are_not() {
        let root = std::env::temp_dir().join(format!("klick-kswatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Versions").join("version-1")).unwrap();
        std::fs::write(root.join("Versions").join("version-1").join("Game.exe"), b"1").unwrap();
        std::fs::write(root.join("log.txt"), b"a").unwrap();

        let (tx, mut rx) = mpsc::channel(16);
        let mut w = KsWatch::new(tx).unwrap();
        w.set(vec![root.to_string_lossy().into_owned()]);
        assert_eq!(w.watching.len(), 1);

        // Игра пишет журнал — не повод пересматривать папку.
        std::fs::write(root.join("log.txt"), b"ab").unwrap();
        assert!(!got(&mut rx, Duration::from_millis(600)));

        // Обновление: новая папка версии с exe.
        let v2 = root.join("Versions").join("version-2");
        std::fs::create_dir_all(&v2).unwrap();
        std::fs::write(v2.join("Game.exe"), b"2").unwrap();
        assert!(got(&mut rx, Duration::from_secs(2)));

        // Готовую папку переложили целиком (так ставит часть установщиков).
        let staged = std::env::temp_dir().join(format!("klick-kswatch-staged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staged);
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(staged.join("Game.exe"), b"3").unwrap();
        std::fs::rename(&staged, root.join("Versions").join("version-3")).unwrap();
        assert!(got(&mut rx, Duration::from_secs(2)));

        // Тот же список — слежку не пересоздаём; пустой — перестаём следить.
        w.set(vec![root.to_string_lossy().into_owned()]);
        assert_eq!(w.watching.len(), 1);
        w.set(Vec::new());
        assert!(w.watching.is_empty());
        std::fs::write(root.join("Versions").join("version-2").join("Other.exe"), b"4").unwrap();
        assert!(!got(&mut rx, Duration::from_millis(600)));
        drop(w);
        let _ = std::fs::remove_dir_all(&root);
    }
}
