<?php
$page_title = '🏢 Enterprise Licensing — WolfStack Docs';
$page_desc = 'WolfStack Enterprise Licensing — full support, installation, ticketing and SLA for businesses';
$active = 'enterprise.php';
$page_css = '.pricing-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
            gap: 1.5rem;
            margin: 2rem 0;
        }

        .pricing-card {
            background: var(--bg-card);
            border: 1px solid var(--border-color);
            border-radius: 14px;
            padding: 2rem 1.5rem;
            text-align: center;
            transition: all 0.3s ease;
            position: relative;
        }

        .pricing-card:hover {
            border-color: rgba(220, 38, 38, 0.3);
            transform: translateY(-4px);
            box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
        }

        .pricing-card.featured {
            border-color: rgba(220, 38, 38, 0.5);
            box-shadow: 0 4px 24px rgba(220, 38, 38, 0.15);
        }

        .pricing-card.featured::before {
            content: \'⭐ Most Popular\';
            position: absolute;
            top: -12px;
            left: 50%;
            transform: translateX(-50%);
            background: linear-gradient(135deg, #dc2626, #ef4444);
            color: white;
            padding: 4px 16px;
            border-radius: 20px;
            font-size: 0.75rem;
            font-weight: 700;
            white-space: nowrap;
        }

        .pricing-tier {
            font-size: 0.8rem;
            font-weight: 600;
            text-transform: uppercase;
            letter-spacing: 0.05em;
            color: var(--text-muted);
            margin-bottom: 0.5rem;
        }

        .pricing-name {
            font-size: 1.3rem;
            font-weight: 800;
            color: var(--text-primary);
            margin-bottom: 0.75rem;
        }

        .pricing-price {
            font-size: 2.2rem;
            font-weight: 800;
            color: var(--accent-primary);
            margin-bottom: 0.25rem;
        }

        .pricing-price .period {
            font-size: 0.85rem;
            font-weight: 500;
            color: var(--text-muted);
        }

        .pricing-price .currency {
            font-size: 1.2rem;
            vertical-align: super;
        }

        .pricing-subtitle {
            font-size: 0.85rem;
            color: var(--text-secondary);
            margin-bottom: 1.5rem;
        }

        .pricing-features {
            list-style: none;
            padding: 0;
            margin: 0 0 1.5rem;
            text-align: left;
        }

        .pricing-features li {
            padding: 0.4rem 0;
            font-size: 0.88rem;
            color: var(--text-secondary);
            border-bottom: 1px solid rgba(255, 255, 255, 0.04);
        }

        .pricing-features li:last-child {
            border-bottom: none;
        }

        .pricing-cta {
            display: inline-block;
            padding: 10px 28px;
            border-radius: 8px;
            font-weight: 700;
            font-size: 0.9rem;
            text-decoration: none;
            transition: all 0.3s ease;
        }

        .pricing-cta-primary {
            background: linear-gradient(135deg, #dc2626, #ef4444);
            color: white;
            box-shadow: 0 4px 15px rgba(220, 38, 38, 0.3);
        }

        .pricing-cta-primary:hover {
            transform: translateY(-2px);
            box-shadow: 0 6px 20px rgba(220, 38, 38, 0.4);
            color: white;
        }

        .pricing-cta-secondary {
            background: var(--bg-tertiary);
            color: var(--text-primary);
            border: 1px solid var(--border-color);
        }

        .pricing-cta-secondary:hover {
            border-color: var(--accent-primary);
            transform: translateY(-2px);
            color: var(--text-primary);
        }

        .pricing-cta-green {
            background: linear-gradient(135deg, #16a34a, #22c55e);
            color: white;
            box-shadow: 0 4px 15px rgba(34, 197, 94, 0.3);
        }

        .pricing-cta-green:hover {
            transform: translateY(-2px);
            box-shadow: 0 6px 20px rgba(34, 197, 94, 0.4);
            color: white;
        }

        .enterprise-includes {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
            gap: 1rem;
            margin: 2rem 0;
        }

        .include-card {
            background: var(--bg-card);
            border: 1px solid var(--border-color);
            border-radius: 10px;
            padding: 1.25rem;
            transition: all 0.3s ease;
        }

        .include-card:hover {
            border-color: rgba(220, 38, 38, 0.2);
        }

        .include-card h4 {
            font-size: 0.95rem;
            font-weight: 700;
            margin-bottom: 0.5rem;
            color: var(--text-primary);
        }

        .include-card p {
            font-size: 0.82rem;
            color: var(--text-secondary);
            line-height: 1.6;
        }

        .contact-box {
            background: linear-gradient(135deg, rgba(220, 38, 38, 0.08), rgba(239, 68, 68, 0.04));
            border: 1px solid rgba(220, 38, 38, 0.2);
            border-radius: 14px;
            padding: 2rem;
            text-align: center;
            margin: 2rem 0;
        }

        .contact-box h3 {
            font-size: 1.2rem;
            font-weight: 700;
            margin-bottom: 0.75rem;
            color: var(--text-primary);
        }

        .contact-box p {
            font-size: 0.9rem;
            color: var(--text-secondary);
            margin-bottom: 1.25rem;
            line-height: 1.7;
        }

        .contact-email {
            display: inline-flex;
            align-items: center;
            gap: 0.5rem;
            padding: 12px 28px;
            background: linear-gradient(135deg, #dc2626, #ef4444);
            color: white;
            text-decoration: none;
            border-radius: 10px;
            font-weight: 700;
            font-size: 1rem;
            box-shadow: 0 4px 20px rgba(220, 38, 38, 0.3);
            transition: all 0.3s ease;
        }

        .contact-email:hover {
            transform: translateY(-2px);
            box-shadow: 0 6px 25px rgba(220, 38, 38, 0.4);
            color: white;
        }

        @media (max-width: 768px) {
            .pricing-grid {
                grid-template-columns: 1fr;
            }

            .enterprise-includes {
                grid-template-columns: 1fr;
            }
        }';
include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">


                <div class="content-section">
                    <h2>How WolfStack is Funded</h2>
                    <p>WolfStack is <strong>free and open-source</strong> software, funded by the community through <a
                            href="https://www.patreon.com/15362110/join" target="_blank"
                            style="color:#22c55e;font-weight:600;">Patreon donations</a>. This keeps it accessible to
                        everyone — from hobbyists running a single server to large organisations managing entire fleets.
                    </p>
                    <p>For businesses that need <strong>guaranteed support, professional installation, and a ticketing
                            system</strong>, we offer Enterprise Licensing. You get everything in the community edition,
                        plus dedicated support from the team that builds it.</p>
                </div>

                <!-- Pricing Cards -->
                <div class="content-section">
                    <h2>Plans &amp; Pricing</h2>
                    <p style="color: var(--text-secondary); font-size: 0.9rem; margin-bottom: 0.5rem;">All enterprise
                        plans are priced <strong>per CPU socket per year</strong>. It doesn't matter how many cores your
                        CPU has — each occupied socket counts as one.</p>
                    <div class="pricing-grid">

                        <!-- Community -->
                        <div class="pricing-card">
                            <div class="pricing-tier">Community</div>
                            <div class="pricing-name">Community Edition</div>
                            <div class="pricing-price" style="color:#22c55e;">Free</div>
                            <div class="pricing-subtitle">Supported by Patreon</div>
                            <ul class="pricing-features">
                                <li>✅ All features included</li>
                                <li>✅ Unlimited CPU sockets</li>
                                <li>✅ Full source code access</li>
                                <li>💬 Community support (Discord &amp; GitHub)</li>
                                <li>📖 Documentation &amp; guides</li>
                                <li>🔄 Regular updates</li>
                            </ul>
                            <a href="https://www.patreon.com/15362110/join" target="_blank"
                                class="pricing-cta pricing-cta-green">Support on Patreon</a>
                        </div>

                        <!-- Basic -->
                        <div class="pricing-card">
                            <div class="pricing-tier">Enterprise</div>
                            <div class="pricing-name">Basic</div>
                            <div class="pricing-price"><span class="currency">£</span>95 <span class="period">/ year
                                    &amp; CPU socket</span></div>
                            <div class="pricing-subtitle">For growing businesses</div>
                            <ul class="pricing-features">
                                <li>✅ Everything in Community</li>
                                <li>📧 Email support</li>
                                <li>🔧 Installation assistance</li>
                                <li>🎫 3 support tickets / year</li>
                                <li>📞 Response within 1 business day</li>
                                <li>📋 Quarterly check-ins</li>
                            </ul>
                            <a href="enterprise-contact.php?plan=Basic"
                                class="pricing-cta pricing-cta-secondary">Contact Sales</a>
                        </div>

                        <!-- Standard -->
                        <div class="pricing-card featured">
                            <div class="pricing-tier">Enterprise</div>
                            <div class="pricing-name">Standard</div>
                            <div class="pricing-price"><span class="currency">£</span>450 <span class="period">/ year
                                    &amp; CPU socket</span></div>
                            <div class="pricing-subtitle">Most popular</div>
                            <ul class="pricing-features">
                                <li>✅ Everything in Basic</li>
                                <li>🚀 Priority support</li>
                                <li>🔧 Full installation &amp; onboarding</li>
                                <li>🎫 10 support tickets / year</li>
                                <li>📞 Response within 4 hours</li>
                                <li>🔄 Monthly check-ins</li>
                                <li>📦 Migration assistance</li>
                            </ul>
                            <a href="enterprise-contact.php?plan=Standard"
                                class="pricing-cta pricing-cta-primary">Contact Sales</a>
                        </div>

                        <!-- Premium -->
                        <div class="pricing-card">
                            <div class="pricing-tier">Enterprise</div>
                            <div class="pricing-name">Premium</div>
                            <div class="pricing-price"><span class="currency">£</span>900 <span class="period">/ year
                                    &amp; CPU socket</span></div>
                            <div class="pricing-subtitle">All you'll ever need</div>
                            <ul class="pricing-features">
                                <li>✅ Everything in Standard</li>
                                <li>⚡ 24/7 support</li>
                                <li>📄 SLA guarantee</li>
                                <li>🎫 Unlimited support tickets</li>
                                <li>📞 Response within 2 hours</li>
                                <li>🤝 Dedicated account manager</li>
                                <li>🎓 Team training sessions</li>
                                <li>🔧 Custom development requests</li>
                            </ul>
                            <a href="enterprise-contact.php?plan=Premium"
                                class="pricing-cta pricing-cta-secondary">Contact Sales</a>
                        </div>

                    </div>
                </div>

                <!-- What's Included -->
                <div class="content-section">
                    <h2>What's Included with Enterprise</h2>
                    <div class="enterprise-includes">
                        <div class="include-card">
                            <h4>🔧 Full Installation</h4>
                            <p>Our team will install and configure WolfStack across your entire infrastructure. We
                                handle the setup so you can focus on your business.</p>
                        </div>
                        <div class="include-card">
                            <h4>🎫 Ticketing System</h4>
                            <p>Dedicated ticketing system for reporting issues, requesting features, and tracking
                                resolution. No more waiting on community forums.</p>
                        </div>
                        <div class="include-card">
                            <h4>📧 Direct Support</h4>
                            <p>Email and priority support directly from the WolfStack development team. Get answers from
                                the people who build the software.</p>
                        </div>
                        <div class="include-card">
                            <h4>🔄 Migration Help</h4>
                            <p>Moving from Proxmox, Portainer, or another platform? We'll help migrate your containers,
                                configurations, and workflows.</p>
                        </div>
                        <div class="include-card">
                            <h4>📄 SLA Guarantee</h4>
                            <p>Enterprise Premium customers receive a Service Level Agreement with guaranteed response
                                times and uptime commitments.</p>
                        </div>
                        <div class="include-card">
                            <h4>🎓 Training</h4>
                            <p>Get your team up to speed with dedicated training sessions covering WolfStack, WolfNet,
                                container management, and more.</p>
                        </div>
                    </div>
                </div>

                <!-- Contact Box -->
                <div class="contact-box">
                    <h3>📧 Ready to Get Started?</h3>
                    <p>Contact our sales team to discuss your requirements and find the right plan for your
                        organisation.<br>
                        We'll get back to you within one business day.</p>
                    <a href="enterprise-contact.php" class="contact-email">
                        ✉️ Contact Sales
                    </a>
                </div>

                <!-- FAQ -->
                <div class="content-section">
                    <h2>Frequently Asked Questions</h2>

                    <h3>How many subscriptions do I need?</h3>
                    <p>Enterprise subscriptions are priced per <strong>physical CPU socket</strong> occupied on the
                        motherboard. It doesn't matter if your CPU has 4 cores or 128 cores — each occupied socket
                        counts as one. Empty slots do not require a subscription. Each server needs its own subscription
                        based on its socket count.</p>

                    <h3>Do I need an enterprise licence to use WolfStack?</h3>
                    <p>No. WolfStack is completely free and open-source under the FSL-1.1 licence. The Community Edition
                        includes all features with no limits. Enterprise licensing is for organisations that want
                        dedicated support and professional services.</p>

                    <h3>Can I switch plans later?</h3>
                    <p>Yes — you can upgrade or downgrade your enterprise plan at any time. Contact <a
                            href="mailto:sales@wolf.uk.com">sales@wolf.uk.com</a> to discuss changes.</p>

                    <h3>What payment methods do you accept?</h3>
                    <p>We accept bank transfer and invoice-based payments. Contact our sales team for details.</p>

                    <h3>Is support available outside business hours?</h3>
                    <p>24/7 support is included with the Enterprise Premium plan. Basic and Standard plans
                        include support during UK business hours (Mon–Fri, 9am–5pm GMT).</p>

                    <h3>Can I still support via Patreon if I have an enterprise licence?</h3>
                    <p>Absolutely! Patreon support is always welcome and helps fund continued development for the entire
                        community.</p>
                </div>

                <div class="page-nav"><a href="licensing.php" class="prev">&larr; Licensing</a><a href="support.php"
                        class="next">Support Us &rarr;</a></div>
            
    </main>
<?php include 'includes/footer.php'; ?>
