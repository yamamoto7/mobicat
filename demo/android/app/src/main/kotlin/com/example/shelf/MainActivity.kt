package com.example.shelf

import android.app.Activity
import android.content.Intent
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView

/**
 * A tiny shopping list that writes the kind of logs a real app does:
 * network calls, cache and cart updates, image loads, and a crash on checkout.
 * The demo recorder drives it with deep links (shelf://sync, shelf://add?item=N,
 * shelf://checkout) so every take is the same.
 */
class MainActivity : Activity() {
    private val main = Handler(Looper.getMainLooper())
    private val cart = Cart()
    private val api = Api()
    private val checkout = CheckoutService()
    private val addButtons = mutableListOf<Button>()
    private lateinit var cartLabel: TextView
    private lateinit var syncButton: Button
    private lateinit var checkoutButton: Button
    private var images = 0

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Log.i("ShelfApp", "onCreate: Shelf 1.0 (build 1)")
        setContentView(layout())
        Log.d("ItemStore", "loaded ${ITEMS.size} items from cache")
        main.postDelayed(imageTick, 1200)
        handle(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    override fun onDestroy() {
        main.removeCallbacks(imageTick)
        super.onDestroy()
    }

    private fun handle(intent: Intent?) {
        val uri = intent?.data ?: return
        when (uri.host) {
            "sync" -> tap(syncButton)
            "add" -> uri.getQueryParameter("item")?.toIntOrNull()
                ?.let { addButtons.getOrNull(it) }
                ?.let { tap(it) }
            "checkout" -> tap(checkoutButton)
        }
    }

    /** Show the press, then click, so the recording looks like a real tap. */
    private fun tap(v: View) {
        v.isPressed = true
        main.postDelayed({
            v.isPressed = false
            v.performClick()
        }, 180)
    }

    private val imageTick = object : Runnable {
        override fun run() {
            val item = ITEMS[images % ITEMS.size]
            val kb = 18 + (images * 7) % 40
            Log.v("ImageLoader", "${item.slug}.webp ${kb} KB (${if (images < ITEMS.size) "network" else "memory cache"})")
            images++
            main.postDelayed(this, 5200)
        }
    }

    private fun sync() {
        Log.i("SyncWorker", "sync started")
        api.get("/v1/items") { ok ->
            if (ok) {
                Log.d("ItemStore", "saved ${ITEMS.size} items")
                Log.i("SyncWorker", "sync finished")
            }
        }
    }

    private fun add(i: Int) {
        val item = ITEMS[i]
        cart.add(item)
        Log.i("CartRepository", "added ${item.name} (cart: ${plural(cart.size)}, ${money(cart.total)})")
        cartLabel.text = "Cart · ${plural(cart.size)} · ${money(cart.total)}"
    }

    private fun checkout() {
        Log.i("CheckoutService", "submitting order (${plural(cart.size)}, ${money(cart.total)})")
        checkout.submit(cart)
    }

    private inner class Api {
        private var calls = 0

        fun get(path: String, done: (Boolean) -> Unit) {
            val n = ++calls
            Log.d("OkHttp", "--> GET https://api.example.com$path")
            if (n == 2) {
                // The second call is slow, like a flaky network.
                main.postDelayed({
                    Log.w("OkHttp", "<-- timeout after 10s, retrying (1/3)")
                    main.postDelayed({
                        Log.d("OkHttp", "<-- 200 OK https://api.example.com$path (412ms, 3.1 KB)")
                        done(true)
                    }, 600)
                }, 900)
            } else {
                main.postDelayed({
                    Log.d("OkHttp", "<-- 200 OK https://api.example.com$path (124ms, 3.1 KB)")
                    done(true)
                }, 300)
            }
        }
    }

    // ---- UI -------------------------------------------------------------

    private fun layout(): View {
        val d = resources.displayMetrics.density
        fun dp(v: Int) = (v * d).toInt()
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.rgb(246, 247, 245))
            setPadding(dp(20), dp(48), dp(20), dp(24))
        }
        root.addView(TextView(this).apply {
            text = "Shelf"
            textSize = 30f
            typeface = Typeface.DEFAULT_BOLD
            setTextColor(INK)
        })
        cartLabel = TextView(this).apply {
            text = "Cart · 0 items"
            textSize = 15f
            setTextColor(Color.rgb(94, 105, 100))
            setPadding(0, dp(4), 0, dp(16))
        }
        root.addView(cartLabel)

        val list = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        ITEMS.forEachIndexed { i, item ->
            val row = LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                setPadding(dp(16), dp(12), dp(10), dp(12))
                background = rounded(Color.WHITE, dp(14))
            }
            row.addView(TextView(this).apply {
                text = "${item.name}\n${money(item.price)}"
                textSize = 16f
                setTextColor(INK)
            }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            val add = pill("Add", ACCENT, dp(10)) { add(i) }
            addButtons += add
            row.addView(add)
            list.addView(row, LinearLayout.LayoutParams(-1, -2).apply { bottomMargin = dp(10) })
        }
        root.addView(ScrollView(this).apply { addView(list) }, LinearLayout.LayoutParams(-1, 0, 1f))

        val bar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(0, dp(12), 0, 0)
        }
        syncButton = pill("Sync", Color.rgb(94, 105, 100), dp(14)) { sync() }
        checkoutButton = pill("Checkout", INK, dp(14)) { checkout() }
        bar.addView(syncButton, LinearLayout.LayoutParams(0, dp(56), 1f).apply { rightMargin = dp(10) })
        bar.addView(checkoutButton, LinearLayout.LayoutParams(0, dp(56), 2f))
        root.addView(bar)
        return root
    }

    private fun pill(label: String, color: Int, radius: Int, onClick: () -> Unit) = Button(this).apply {
        text = label
        isAllCaps = false
        textSize = 16f
        setTextColor(Color.WHITE)
        background = android.graphics.drawable.StateListDrawable().apply {
            addState(intArrayOf(android.R.attr.state_pressed), rounded(darker(color), radius))
            addState(intArrayOf(), rounded(color, radius))
        }
        stateListAnimator = null
        setOnClickListener { onClick() }
    }

    private fun rounded(color: Int, radius: Int) = GradientDrawable().apply {
        setColor(color)
        cornerRadius = radius.toFloat()
    }

    private fun darker(c: Int) = Color.rgb(Color.red(c) * 6 / 10, Color.green(c) * 6 / 10, Color.blue(c) * 6 / 10)

    companion object {
        val INK = Color.rgb(28, 34, 32)
        val ACCENT = Color.rgb(47, 125, 91)
    }
}

data class Item(val name: String, val slug: String, val price: Int)

val ITEMS = listOf(
    Item("Oat milk", "oat-milk", 249),
    Item("Sourdough loaf", "sourdough", 420),
    Item("Eggs (12)", "eggs", 380),
    Item("Coffee beans", "coffee", 1190),
    Item("Bananas", "bananas", 199),
    Item("Greek yogurt", "yogurt", 275),
)

fun plural(n: Int) = if (n == 1) "1 item" else "$n items"

fun money(cents: Int) = "$%d.%02d".format(cents / 100, cents % 100)

class Cart {
    private val items = mutableListOf<Item>()
    val size get() = items.size
    val total get() = items.sumOf { it.price }
    fun add(item: Item) = items.add(item)
    fun snapshot() = items.toList()
}

class CheckoutService {
    fun submit(cart: Cart) {
        val order = cart.snapshot()
        validate(order, expected = cart.size + 1)
    }

    // The bug the demo shows: the expected count is off by one.
    private fun validate(order: List<Item>, expected: Int) {
        check(order.size == expected) {
            "Cart changed during checkout: expected $expected items, found ${order.size}"
        }
    }
}
