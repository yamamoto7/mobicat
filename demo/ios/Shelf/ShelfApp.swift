import SwiftUI
import os

// A tiny shopping list that writes the kind of logs a real app does:
// network calls, cache and cart updates, image loads, and a crash on checkout.
// The demo recorder drives it with Darwin notifications, so every take is the
// same and no "Open in Shelf?" prompt appears:
//   xcrun simctl spawn booted notifyutil -p com.example.shelf.sync
//   (also .add0 … .add5 and .checkout)

enum Log {
    static let app = Logger(subsystem: "com.example.shelf", category: "app")
    static let network = Logger(subsystem: "com.example.shelf", category: "network")
    static let store = Logger(subsystem: "com.example.shelf", category: "store")
    static let cart = Logger(subsystem: "com.example.shelf", category: "cart")
    static let images = Logger(subsystem: "com.example.shelf", category: "images")
    static let checkout = Logger(subsystem: "com.example.shelf", category: "checkout")
}

struct Item: Identifiable {
    let id: Int
    let name: String
    let slug: String
    let price: Int
}

let items = [
    Item(id: 0, name: "Oat milk", slug: "oat-milk", price: 249),
    Item(id: 1, name: "Sourdough loaf", slug: "sourdough", price: 420),
    Item(id: 2, name: "Eggs (12)", slug: "eggs", price: 380),
    Item(id: 3, name: "Coffee beans", slug: "coffee", price: 1190),
    Item(id: 4, name: "Bananas", slug: "bananas", price: 199),
    Item(id: 5, name: "Greek yogurt", slug: "yogurt", price: 275),
]

func plural(_ n: Int) -> String { n == 1 ? "1 item" : "\(n) items" }

func money(_ cents: Int) -> String { String(format: "$%d.%02d", cents / 100, cents % 100) }

@MainActor
final class Model: ObservableObject {
    @Published var cart: [Item] = []
    @Published var pressed: String?
    private var calls = 0
    private var images = 0
    private var timer: Timer?

    var total: Int { cart.reduce(0) { $0 + $1.price } }

    init() {
        Log.app.info("launched: Shelf 1.0 (build 1)")
        Log.store.debug("loaded \(items.count) items from cache")
        listen()
        timer = Timer.scheduledTimer(withTimeInterval: 5.2, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.imageTick() }
        }
    }

    private func imageTick() {
        let item = items[images % items.count]
        let kb = 18 + (images * 7) % 40
        let from = images < items.count ? "network" : "memory cache"
        Log.images.debug("\(item.slug).webp \(kb) KB (\(from))")
        images += 1
    }

    private func listen() {
        let names = ["sync", "checkout"] + items.map { "add\($0.id)" }
        let center = CFNotificationCenterGetDarwinNotifyCenter()
        let me = Unmanaged.passUnretained(self).toOpaque()
        for n in names {
            CFNotificationCenterAddObserver(center, me, { _, observer, name, _, _ in
                guard let observer, let name else { return }
                let model = Unmanaged<Model>.fromOpaque(observer).takeUnretainedValue()
                let id = (name.rawValue as String).replacingOccurrences(of: "com.example.shelf.", with: "")
                DispatchQueue.main.async { MainActor.assumeIsolated { model.run(id) } }
            }, "com.example.shelf.\(n)" as CFString, nil, .deliverImmediately)
        }
    }

    func run(_ id: String) {
        switch id {
        case "sync": tap(id, sync)
        case "checkout": tap(id, checkout)
        default:
            if id.hasPrefix("add"), let i = Int(id.dropFirst(3)), items.indices.contains(i) {
                tap(id) { self.add(items[i]) }
            }
        }
    }

    /// Show the press, then act, so the recording looks like a real tap.
    private func tap(_ id: String, _ action: @escaping () -> Void) {
        pressed = id
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.18) {
            self.pressed = nil
            action()
        }
    }

    func add(_ item: Item) {
        cart.append(item)
        Log.cart.info("added \(item.name) (cart: \(plural(self.cart.count)), \(money(self.total)))")
    }

    func sync() {
        calls += 1
        let n = calls
        Log.network.info("GET https://api.example.com/v1/items")
        if n == 2 {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.9) {
                Log.network.error("request timed out after 10s, retrying (1/3)")
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) { self.synced(ms: 412) }
            }
        } else {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { self.synced(ms: 124) }
        }
    }

    private func synced(ms: Int) {
        Log.network.info("200 OK /v1/items (\(ms) ms, 3.1 KB)")
        Log.store.debug("saved \(items.count) items")
    }

    func checkout() {
        Log.checkout.info("submitting order (\(plural(self.cart.count)), \(money(self.total)))")
        let order = CheckoutService().makeOrder(cart)
        Log.checkout.info("order \(order.id) placed")
    }
}

struct Order { let id: String }

struct CheckoutService {
    // The bug the demo shows: the payment token is never loaded.
    var paymentToken: String?

    func makeOrder(_ cart: [Item]) -> Order {
        Order(id: "\(paymentToken!.prefix(6))-\(cart.count)")
    }
}

@main
struct ShelfApp: App {
    @StateObject private var model = Model()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(model)
        }
    }
}

struct ContentView: View {
    @EnvironmentObject var model: Model
    private let ink = Color(red: 28 / 255, green: 34 / 255, blue: 32 / 255)
    private let accent = Color(red: 47 / 255, green: 125 / 255, blue: 91 / 255)
    private let muted = Color(red: 94 / 255, green: 105 / 255, blue: 100 / 255)

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Shelf").font(.system(size: 34, weight: .bold)).foregroundStyle(ink)
            Text(model.cart.isEmpty ? "Cart · 0 items" : "Cart · \(plural(model.cart.count)) · \(money(model.total))")
                .font(.system(size: 16)).foregroundStyle(muted)
                .padding(.top, 4).padding(.bottom, 16)
            ScrollView {
                VStack(spacing: 10) {
                    ForEach(items) { item in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(item.name).font(.system(size: 17)).foregroundStyle(ink)
                                Text(money(item.price)).font(.system(size: 15)).foregroundStyle(muted)
                            }
                            Spacer()
                            button("Add", id: "add\(item.id)", color: accent, radius: 10, height: 40) { model.add(item) }
                                .frame(width: 76)
                        }
                        .padding(.horizontal, 16).padding(.vertical, 12)
                        .background(Color.white, in: RoundedRectangle(cornerRadius: 14))
                    }
                }
            }
            HStack(spacing: 10) {
                button("Sync", id: "sync", color: muted, radius: 14, height: 56) { model.sync() }
                    .frame(width: 120)
                button("Checkout", id: "checkout", color: ink, radius: 14, height: 56) { model.checkout() }
            }
            .padding(.top, 12)
        }
        .padding(.horizontal, 20).padding(.top, 8)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color(red: 246 / 255, green: 247 / 255, blue: 245 / 255).ignoresSafeArea())
    }

    private func button(_ label: String, id: String, color: Color, radius: CGFloat, height: CGFloat,
                        action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(label).font(.system(size: 17, weight: .medium)).foregroundStyle(.white)
                .frame(maxWidth: .infinity).frame(height: height)
                .background(model.pressed == id ? color.opacity(0.6) : color, in: RoundedRectangle(cornerRadius: radius))
        }
        .buttonStyle(.plain)
    }
}
