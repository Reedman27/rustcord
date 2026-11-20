// Qt WebEngine window for Rustcord. Policy (navigation rules, settings, themes, mods,
// presence) lives in Rust; this file only owns the window, profile and Qt plumbing.
// No Q_OBJECT classes, so no moc step is needed.
#include <QApplication>
#include <QRegularExpression>
#include <QFileInfo>
#include <QCloseEvent>
#include <QDesktopServices>
#include <QDir>
#include <QIcon>
#include <QMenu>
#include <QPixmap>
#include <QPointer>
#include <QStandardPaths>
#include <QSystemTrayIcon>
#include <QTimer>
#include <QUrl>
#include <QWebEngineDownloadRequest>
#include <QWebEngineFullScreenRequest>
#include <QWebEnginePage>
#include <QWebEngineProfile>
#include <QWebEngineScript>
#include <QWebEngineScriptCollection>
#include <QWebEngineSettings>
#include <QWebEngineView>

extern "C" {
struct RcConfig {
    const char *start_url;
    const char *profile_dir;
    const char *chromium_flags;
    const char *mod_script;  // empty = none
    const char *theme_script;  // empty = none
    const unsigned char *icon_png;
    unsigned long icon_len;
    int x, y, w, h;
    int has_pos, maximized, start_minimized, tray, close_to_tray, sandbox;
};
// Implemented in Rust.
int rc_nav_allowed(const char *url);
void rc_open_external(const char *url);
void rc_save_window(int x, int y, int w, int h, int maximized);
}

namespace {

QString g_modScript, g_themeScript;
bool g_closeToTray = false;
QSystemTrayIcon *g_tray = nullptr;
bool g_quitting = false;
QIcon g_icon;

bool navAllowed(const QUrl &u) {
    return rc_nav_allowed(u.toString(QUrl::FullyEncoded).toUtf8().constData()) != 0;
}

void openExternal(const QUrl &u) {
    rc_open_external(u.toString(QUrl::FullyEncoded).toUtf8().constData());
}

bool isDiscordHost(const QUrl &u) {
    return u.scheme() == "https" && (u.host() == "discord.com" || u.host().endsWith(".discord.com"));
}

class View;
class Page : public QWebEnginePage {
public:
    QPointer<QWebEngineView> owner;
    Page(QWebEngineProfile *p, QObject *parent) : QWebEnginePage(p, parent) {
        connect(this, &QWebEnginePage::featurePermissionRequested, this,
                [this](const QUrl &origin, QWebEnginePage::Feature f) { onPermission(origin, f); });
        connect(this, &QWebEnginePage::fullScreenRequested, this,
                [this](QWebEngineFullScreenRequest r) {
                    r.accept();
                    QWidget *w = owner ? owner->window() : nullptr;
                    if (!w) return;
                    if (r.toggleOn()) w->showFullScreen(); else w->showNormal();
                });
    }

    bool acceptNavigationRequest(const QUrl &url, NavigationType, bool isMainFrame) override {
        if (!isMainFrame) return true;
        if (navAllowed(url)) return true;
        openExternal(url);
        return false;
    }

    QWebEnginePage *createWindow(WebWindowType type) override;

private:
    void onPermission(const QUrl &origin, Feature f) {
        bool media = f == MediaAudioCapture || f == MediaVideoCapture ||
                     f == MediaAudioVideoCapture || f == DesktopVideoCapture ||
                     f == DesktopAudioVideoCapture || f == Notifications;
        if (media && isDiscordHost(origin))
            setFeaturePermission(origin, f, PermissionGrantedByUser);
        else
            setFeaturePermission(origin, f, PermissionDeniedByUser);
    }
};

class View : public QWebEngineView {
public:
    bool main;
    View(QWebEngineProfile *p, bool isMain) : QWebEngineView(), main(isMain) {
        Page *pg = new Page(p, this);
        pg->owner = this;
        setPage(pg);
        setAttribute(Qt::WA_DeleteOnClose, !isMain);
        setWindowIcon(g_icon);
    }

    void saveState() {
        if (!main) return;
        bool max = isMaximized();
        QRect g = max ? normalGeometry() : geometry();
        rc_save_window(g.x(), g.y(), g.width(), g.height(), max ? 1 : 0);
    }

    QWebEngineView *popup();

protected:
    void closeEvent(QCloseEvent *e) override {
        if (main) {
            saveState();
            if (g_closeToTray && g_tray && g_tray->isVisible() && !g_quitting) {
                hide();
                e->ignore();
                return;
            }
            g_quitting = true;
            QApplication::quit();
        }
        QWebEngineView::closeEvent(e);
    }

    QWebEngineView *createWindow(QWebEnginePage::WebWindowType) override { return popup(); }
};

QWebEngineProfile *g_profile = nullptr;
QPointer<View> g_main;

QWebEnginePage *Page::createWindow(WebWindowType type) {
    (void)type;
    View *v = static_cast<View *>(owner.data());
    QWebEngineView *w = v ? v->popup() : nullptr;
    return w ? w->page() : nullptr;
}

// Discord pop-outs open about:blank then navigate in-app; other targets go to the browser
// (acceptNavigationRequest sends them out) and the empty window closes itself.
QWebEngineView *View::popup() {
    View *pop = new View(g_profile, false);
    pop->resize(800, 600);
    pop->show();
    QPointer<View> guard(pop);
    QTimer::singleShot(4000, pop, [guard]() {
        if (!guard) return;
        QString u = guard->url().toString();
        if (u.isEmpty() || (u == "about:blank" && guard->title().isEmpty())) guard->close();
    });
    return pop;
}

void showMain() {
    if (!g_main) return;
    g_main->showNormal();
    g_main->raise();
    g_main->activateWindow();
}

}  // namespace

extern "C" {

int rc_qt_run(int argc, char **argv, const RcConfig *c) {
    qputenv("QTWEBENGINE_CHROMIUM_FLAGS", c->chromium_flags);
    if (!c->sandbox) qputenv("QTWEBENGINE_DISABLE_SANDBOX", "1");
    QCoreApplication::setAttribute(Qt::AA_ShareOpenGLContexts);
    QApplication app(argc, argv);
    QApplication::setApplicationName("rustcord");
    QApplication::setOrganizationName("rustcord");
    QApplication::setDesktopFileName("rustcord");
    QApplication::setQuitOnLastWindowClosed(false);

    QPixmap pm;
    pm.loadFromData(c->icon_png, (uint)c->icon_len, "PNG");
    g_icon = QIcon(pm);
    app.setWindowIcon(g_icon);
    g_modScript = QString::fromUtf8(c->mod_script);
    g_themeScript = QString::fromUtf8(c->theme_script);
    g_closeToTray = c->close_to_tray != 0;

    QString dir = QString::fromUtf8(c->profile_dir);
    QDir().mkpath(dir);
    g_profile = new QWebEngineProfile("rustcord", &app);
    g_profile->setPersistentStoragePath(dir + "/qt-data");
    g_profile->setCachePath(dir + "/qt-cache");
    g_profile->setPersistentCookiesPolicy(QWebEngineProfile::ForcePersistentCookies);
    g_profile->setHttpCacheType(QWebEngineProfile::DiskHttpCache);
    // Present as plain Chromium: drop the QtWebEngine token from the user agent.
    QString ua = g_profile->httpUserAgent();
    ua.replace(QRegularExpression(" QtWebEngine/[0-9.]+"), "");
    g_profile->setHttpUserAgent(ua);

    QWebEngineSettings *s = g_profile->settings();
    s->setAttribute(QWebEngineSettings::PlaybackRequiresUserGesture, false);
    s->setAttribute(QWebEngineSettings::FullScreenSupportEnabled, true);
    s->setAttribute(QWebEngineSettings::ScreenCaptureEnabled, true);
    s->setAttribute(QWebEngineSettings::JavascriptCanOpenWindows, true);

    auto addScript = [](const QString &name, const QString &src, QWebEngineScript::InjectionPoint ip) {
        QWebEngineScript sc;
        sc.setName(name);
        sc.setSourceCode(src);
        sc.setInjectionPoint(ip);
        sc.setWorldId(QWebEngineScript::MainWorld);
        sc.setRunsOnSubFrames(false);
        g_profile->scripts()->insert(sc);
    };
    const QString guardOpen = "if(/(^|\\.)discord\\.com$/.test(location.hostname)){";
    if (!g_modScript.isEmpty())
        addScript("rustcord-mod", guardOpen + g_modScript + "\n}", QWebEngineScript::DocumentCreation);
    if (!g_themeScript.isEmpty())
        addScript("rustcord-theme", guardOpen + g_themeScript + "\n}", QWebEngineScript::DocumentReady);

    // Downloads go to the user's Downloads folder without overwriting existing files.
    QObject::connect(g_profile, &QWebEngineProfile::downloadRequested, [](QWebEngineDownloadRequest *d) {
        QString base = QStandardPaths::writableLocation(QStandardPaths::DownloadLocation);
        QDir().mkpath(base);
        QString name = d->downloadFileName().isEmpty() ? "download" : d->downloadFileName();
        QFileInfo fi(name);
        QString path = base + "/" + fi.fileName();
        for (int i = 1; QFileInfo::exists(path); ++i)
            path = base + "/" + fi.completeBaseName() + QString(" (%1)").arg(i) +
                   (fi.suffix().isEmpty() ? "" : "." + fi.suffix());
        d->setDownloadDirectory(QFileInfo(path).absolutePath());
        d->setDownloadFileName(QFileInfo(path).fileName());
        d->accept();
    });

    View *v = new View(g_profile, true);
    g_main = v;
    v->setWindowTitle("Rustcord");
    v->resize(c->w, c->h);
    if (c->has_pos) v->move(c->x, c->y);
    QObject::connect(v->page(), &QWebEnginePage::titleChanged, v, [v](const QString &t) {
        v->setWindowTitle(t.isEmpty() ? "Rustcord" : t);
    });
    v->load(QUrl(QString::fromUtf8(c->start_url)));
    v->setMinimumSize(940, 500);
    if (c->start_minimized) v->showMinimized();
    else if (c->maximized) v->showMaximized();
    else v->show();

    if (c->tray && QSystemTrayIcon::isSystemTrayAvailable()) {
        g_tray = new QSystemTrayIcon(g_icon, &app);
        QMenu *menu = new QMenu();
        menu->addAction("Show Rustcord", [] { showMain(); });
        menu->addAction("Quit", [] {
            g_quitting = true;
            if (g_main) g_main->saveState();
            QApplication::quit();
        });
        g_tray->setContextMenu(menu);
        g_tray->setToolTip("Rustcord");
        QObject::connect(g_tray, &QSystemTrayIcon::activated, [](QSystemTrayIcon::ActivationReason r) {
            if (r == QSystemTrayIcon::Trigger) showMain();
        });
        g_tray->show();
    }

    int rc = app.exec();
    delete g_main.data();
    return rc;
}

// Thread-safe: called from the single-instance thread.
void rc_qt_show() {
    QMetaObject::invokeMethod(qApp, [] { showMain(); }, Qt::QueuedConnection);
}
void rc_qt_open(const char *url) {
    QString u = QString::fromUtf8(url);
    QMetaObject::invokeMethod(qApp, [u] {
        if (g_main) g_main->load(QUrl(u));
        showMain();
    }, Qt::QueuedConnection);
}
}
